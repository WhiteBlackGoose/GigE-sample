#![feature(cursor_remaining)]
use std::{io::Cursor, net::Ipv4Addr, str::FromStr, time::Instant};

use cameleon::{
    gige::{ControlHandle, StreamHandle},
    payload::{ImageInfo, Payload, PayloadReceiver},
    Camera,
};
use cameleon_device::PixelFormat;
use egui::{ColorImage, Label, TextEdit, TextureHandle};
use image::{ImageBuffer, Rgb};

#[tokio::main]
async fn main() {
    let filter = tracing_subscriber::EnvFilter::from_default_env();
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 960.0]),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "GigE 💕 cameleon example",
        options,
        Box::new(|cc| {
            // This gives us image support:
            egui_extras::install_image_loaders(&cc.egui_ctx);

            Ok(Box::new(MyApp::new(&cc.egui_ctx)))
        }),
    )
    .unwrap();
}

struct FpsCounter {
    timestamp: std::time::Instant,
    fps_count: u64,
    avg: Option<f64>,
}

impl FpsCounter {
    fn new() -> Self {
        Self {
            timestamp: Instant::now(),
            fps_count: 0,
            avg: None,
        }
    }

    pub fn bump(&mut self) {
        self.fps_count += 1;
        let delta = Instant::now() - self.timestamp;
        if delta.as_secs() > 1 {
            self.avg = Some(self.fps_count as f64 / delta.as_secs_f64());
            self.timestamp = Instant::now();
            self.fps_count = 0;
        }
    }
}

impl std::fmt::Display for FpsCounter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.avg {
            None => f.write_str("N/A"),
            Some(avg) => f.write_fmt(format_args!("{:.02}", avg)),
        }
    }
}

struct MyApp {
    handle: TextureHandle,
    ip_addr: String,
    cam: Option<(Camera<ControlHandle, StreamHandle>, PayloadReceiver)>,
    last_im: Option<ImageInfo>,
    fps: Option<FpsCounter>,
}

impl MyApp {
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            handle: ctx.load_texture(
                "s",
                rgb2egui(&ImageBuffer::from_vec(1, 1, vec![1, 1, 1]).unwrap()),
                egui::TextureOptions::LINEAR,
            ),
            ip_addr: String::from_str("192.168.1.3").unwrap(),
            cam: None,
            last_im: None,
            fps: None,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_pixels_per_point(2.0);
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut self.ip_addr));
                if ui.button("Start").clicked() && self.cam.is_none() {
                    self.cam = Some(get_camera(Ipv4Addr::from_str(&self.ip_addr).unwrap()));
                    self.fps = Some(FpsCounter::new());
                }
                if ui.button("Stop").clicked() && self.cam.is_some() {
                    let (mut cam, _) = self.cam.take().unwrap();
                    cam.stop_streaming().unwrap();
                    cam.close().unwrap();
                    self.cam = None;
                    self.last_im = None;
                    self.fps = None;
                }
                if let Some(im) = self.last_im.as_ref() {
                    ui.add(Label::new(format!(
                        "{}x{} {:?}",
                        im.width, im.height, im.pixel_format
                    )));
                }
                if let Some(fps) = self.fps.as_ref() {
                    ui.add(Label::new(format!("{} fps", fps)));
                }
            });

            let txt = egui::load::SizedTexture::from_handle(&self.handle);
            ui.add(egui::Image::from_texture(txt).shrink_to_fit());

            let Some((_, prx)) = self.cam.as_ref() else {
                return;
            };
            let buf = prx.try_recv();
            let Ok(buf) = buf else {
                return;
            };
            if let Some(fps) = self.fps.as_mut() {
                fps.bump();
            }
            self.last_im = Some(buf.image_info().unwrap().clone());
            let rgb = cameleon2rgb(buf);
            let img = rgb2egui(&rgb);
            self.handle.set(img, egui::TextureOptions::LINEAR);
        });
        ctx.request_repaint();
    }
}

fn cameleon2rgb(buf: Payload) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let mut raw = Cursor::new(buf.payload());
    let mut rgb = vec![0u8; buf.payload().len() * 3];
    let ii = buf.image_info().unwrap();
    assert_eq!(ii.width * ii.height, buf.payload().len());
    assert_eq!(ii.pixel_format, PixelFormat::BayerRG8);
    let mut raster =
        bayer::RasterMut::new(ii.width, ii.height, bayer::RasterDepth::Depth8, &mut rgb);
    bayer::demosaic(
        &mut raw,
        bayer::BayerDepth::Depth8,
        bayer::CFA::RGGB,
        bayer::Demosaic::Linear,
        &mut raster,
    )
    .unwrap();
    let buffer: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_vec(ii.width as u32, ii.height as u32, rgb).unwrap();
    buffer
}

fn rgb2egui(rgb: &ImageBuffer<Rgb<u8>, Vec<u8>>) -> ColorImage {
    egui::ColorImage::from_rgb([rgb.width() as usize, rgb.height() as usize], rgb)
}

fn get_camera(ip_addr: Ipv4Addr) -> (Camera<ControlHandle, StreamHandle>, PayloadReceiver) {
    let mut camera = cameleon::gige::enumerate_cameras(ip_addr)
        .unwrap()
        .swap_remove(0);
    camera.open().unwrap();
    camera.load_context().unwrap();
    let mut ctxt = camera.params_ctxt().unwrap();

    ctxt.node("GainAuto")
        .unwrap()
        .as_enumeration(&ctxt)
        .unwrap()
        .set_entry_by_symbolic(&mut ctxt, "Continuous")
        .unwrap();

    let prx = camera.start_streaming(3).unwrap();
    (camera, prx)
}
