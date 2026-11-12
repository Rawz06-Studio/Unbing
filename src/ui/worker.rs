//! Background work for the UI: network, disk and image decoding.
//! Results come back through `mpsc`; every message requests a repaint.

use eframe::egui::{self, ColorImage};
use image::DynamicImage;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use tracing::warn;

use crate::bing::BingImage;
use crate::cache::Cache;
use crate::config::Config;
use crate::sync::{Engine, Progress, today};

/// Largest preview sent to the GPU (no need for a 4K texture).
const PREVIEW_MAX: (u32, u32) = (1920, 1080);
/// Sidebar thumbnails (×2 for Retina screens).
const THUMB_MAX: (u32, u32) = (160, 90);
/// When the metadata could not be fetched, try again in an hour.
const RETRY_AFTER_ERROR: i64 = 3600;

#[derive(Clone, Copy, Debug)]
pub enum Job {
    /// Launch, new day or new Bing publication: today's picture, applied if the daily change is on.
    Startup,
    /// Market changed: fetch the metadata again and show today's picture.
    Refresh,
    /// Show the picture at this index (downloaded only if it isn't on disk yet).
    Show(usize),
    /// Set the picture at this index as wallpaper.
    Apply(usize),
}

pub enum Msg {
    Status(String),
    Metadata {
        images: Vec<BingImage>,
        offline: bool,
    },
    Preview {
        index: usize,
        id: String,
        image: ColorImage,
        path: PathBuf,
    },
    Thumb {
        id: String,
        image: ColorImage,
    },
    Applied {
        id: Option<String>,
        backend: Option<&'static str>,
    },
    /// Unix time after which a new picture is expected.
    NextDue(i64),
    Requests(u32),
    Failed(String),
    Done,
}

/// Way back to the UI: every message triggers a repaint.
#[derive(Clone)]
pub struct Reporter {
    pub tx: Sender<Msg>,
    pub ctx: egui::Context,
}

impl Reporter {
    pub fn send(&self, msg: Msg) {
        // The window is gone: nobody is listening, nothing to do.
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }
}

pub fn spawn(job: Job, cfg: Config, images: Vec<BingImage>, out: Reporter) -> std::io::Result<()> {
    std::thread::Builder::new().name("unbing-worker".into()).spawn(move || {
        run_job(job, cfg, images, &out);
        out.send(Msg::Done);
    })?;
    Ok(())
}

/// Loads, in the background, the thumbnails of the pictures already on disk (no request).
pub fn spawn_thumbs(cfg: Config, images: Vec<BingImage>, known: Vec<String>, out: Reporter) {
    let result = std::thread::Builder::new().name("unbing-thumbs".into()).spawn(move || {
        let Ok(cache) = Cache::open_default(&cfg) else { return };
        out.send(Msg::Applied { id: cache.applied_id(), backend: None });
        for img in images {
            let id = img.id();
            if known.contains(&id) {
                continue;
            }
            if let Some(path) = cache.lookup(&id, false).or_else(|| cache.lookup(&id, true)) {
                match decode(&path) {
                    Ok(full) => out.send(Msg::Thumb { id, image: to_color_image(&full, THUMB_MAX) }),
                    Err(e) => warn!("{e}"),
                }
            }
        }
    });
    if let Err(e) = result {
        warn!("thumbnails not loaded: {e}");
    }
}

fn run_job(job: Job, cfg: Config, mut images: Vec<BingImage>, out: &Reporter) {
    let auto_apply = cfg.auto_apply;
    let mut engine = match Engine::new(cfg) {
        Ok(e) => e,
        Err(e) => {
            out.send(Msg::Failed(e.to_string()));
            return;
        }
    };
    let mut progress = |p: Progress| {
        out.send(Msg::Status(match p {
            Progress::Metadata => "Looking for today's picture…".into(),
            Progress::Downloading(_) => "Downloading…".into(),
            Progress::Applying => "Setting wallpaper…".into(),
        }))
    };
    let requests = |engine: &Engine| Msg::Requests(engine.cache.requests_on(&today()));

    let (index, apply) = match job {
        Job::Startup | Job::Refresh => {
            match engine.metadata(&mut progress) {
                Ok(list) => {
                    images = list;
                    out.send(Msg::Metadata { images: images.clone(), offline: false });
                    if let Some(due) = engine.next_due() {
                        out.send(Msg::NextDue(due.timestamp()));
                    }
                }
                Err(e) => {
                    // No network: show what we have, never delete anything.
                    warn!("{e}");
                    out.send(Msg::NextDue(chrono::Utc::now().timestamp() + RETRY_AFTER_ERROR));
                    match engine.offline_metadata() {
                        Some(list) => {
                            images = list;
                            out.send(Msg::Metadata { images: images.clone(), offline: true });
                            show_cached(&engine, &images, 0, out);
                            out.send(Msg::Failed(format!("Offline — {e}")));
                        }
                        None => out.send(Msg::Failed(e.to_string())),
                    }
                    out.send(requests(&engine));
                    return;
                }
            }
            (0, matches!(job, Job::Startup) && auto_apply)
        }
        Job::Show(i) => (i, false),
        Job::Apply(i) => (i, true),
    };

    let Some(img) = images.get(index).cloned() else {
        out.send(requests(&engine));
        return;
    };
    let id = img.id();
    let shown = engine.ensure_image(&img, &mut progress).map_err(|e| e.to_string()).and_then(|path| {
        let full = decode(&path)?;
        out.send(Msg::Thumb { id: id.clone(), image: to_color_image(&full, THUMB_MAX) });
        out.send(Msg::Preview { index, id: id.clone(), image: to_color_image(&full, PREVIEW_MAX), path: path.clone() });
        Ok(path)
    });

    match shown {
        Ok(path) => {
            let explicit = matches!(job, Job::Apply(_));
            if apply && (explicit || !engine.is_applied(&path)) {
                progress(Progress::Applying);
                match engine.apply(&path) {
                    Ok(backend) => out.send(Msg::Applied { id: Some(id), backend: Some(backend) }),
                    Err(e) => out.send(Msg::Failed(e.to_string())),
                }
            }
        }
        Err(e) => {
            warn!("{e}");
            out.send(Msg::Failed(e));
        }
    }
    out.send(requests(&engine));
}

fn show_cached(engine: &Engine, images: &[BingImage], index: usize, out: &Reporter) {
    let Some(img) = images.get(index) else { return };
    let Some(path) = engine.cache.lookup(&img.id(), engine.cfg.uhd) else { return };
    match decode(&path) {
        Ok(full) => {
            out.send(Msg::Preview { index, id: img.id(), image: to_color_image(&full, PREVIEW_MAX), path });
        }
        Err(e) => warn!("{e}"),
    }
}

fn decode(path: &Path) -> Result<DynamicImage, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
        .map_err(|e| format!("unreadable picture ({}): {e}", path.display()))
}

fn to_color_image(img: &DynamicImage, max: (u32, u32)) -> ColorImage {
    let rgba = img.thumbnail(max.0, max.1).to_rgba8();
    ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
}

/// Opens the pictures folder in the system file manager.
pub fn open_folder(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let program = "xdg-open";
    if let Err(e) = std::process::Command::new(program).arg(dir).spawn() {
        warn!("cannot open {}: {e}", dir.display());
    }
}
