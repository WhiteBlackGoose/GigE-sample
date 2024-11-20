#![feature(cursor_remaining)]
use std::{io::Cursor, net::Ipv4Addr, str::FromStr};

use cameleon::{
    gige::{ControlHandle, StreamHandle},
    payload::{Payload, PayloadReceiver},
    Camera,
};
use egui::{ColorImage, TextEdit, TextureHandle};
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

struct MyApp {
    handle: TextureHandle,
    ip_addr: String,
    cam: Option<(Camera<ControlHandle, StreamHandle>, PayloadReceiver)>,
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
                }
                if ui.button("Stop").clicked() && self.cam.is_some() {
                    let (mut cam, _) = self.cam.take().unwrap();
                    cam.stop_streaming().unwrap();
                    cam.close().unwrap();
                    self.cam = None;
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
