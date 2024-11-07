// use std::net::UdpSocket;
#![feature(cursor_remaining)]
use std::{io::Cursor, net::UdpSocket, sync::mpsc, thread, time::Duration};

use anyhow::Result;
use cameleon::{
    gige::{enumerate_cameras, register_map::Bootstrap, ControlHandle},
    DeviceControl,
};
use image::{ImageBuffer, Luma, Rgb};
use tokio::{io::AsyncReadExt, runtime};

const CHID: u64 = 0;
const REG_NumOfMessages: u64 = 0x0900;
const REG_NumOfStreams: u64 = 0x0904;
const REG_SCP: u64 = 0x0D00 + 0x40 * CHID; // port (0 for close)
const REG_SCPS: u64 = 0x0D04 + 0x40 * CHID; // packet size
const REG_SCPD: u64 = 0x0D08 + 0x40 * CHID; // packet delay
const REG_SCDA: u64 = 0x0D18 + 0x40 * CHID; // destination IP
const REG_SCSP: u64 = 0x0D1C + 0x40 * CHID; // destination IP
const REG_MCP: u64 = 0x0B00;
const REG_MCDA: u64 = 0x0B10;

fn set_packet_size_and_fire(ch: &mut ControlHandle, target_size: u16) -> u16 {
    ch.write_reg(
        REG_SCPS,
        [
            0b0000_0000,
            0,
            (target_size / 256) as u8,
            (target_size % 256) as u8,
        ],
    )
    .unwrap();
    let [_, _, hi, lo] = ch.read_reg(REG_SCPS).unwrap();
    hi as u16 * 256 + lo as u16
}

fn negotiate_packet_size(ch: &mut ControlHandle) {
    let (got_packet_tx, mut got_packet_rx) = mpsc::channel();
    thread::scope(|s| {
        s.spawn(|| {
            let socket = UdpSocket::bind("0.0.0.0:9998").unwrap();
            let mut buf = [0u8; 65535];
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            loop {
                let r = match socket.recv(&mut buf) {
                    Ok(_) => got_packet_tx.send(Some(())),
                    Err(_) => got_packet_tx.send(None),
                };
                if r.is_err() {
                    return;
                }
            }
        });
        s.spawn(move || {
            let size = set_packet_size_and_fire(ch, 65535);
            tracing::debug!("Set size: {}", size);
            tracing::debug!("Success: {}", got_packet_rx.recv().unwrap().is_some());
            /*
            let mut largest: u16 = 1;
            let mut mid: u16 = 1;

            ch.write_reg(REG_SCPSx, [0, 0, 255, 255]).unwrap();
            let [_, _, upp_hi, upp_lo] = ch.read_reg(REG_SCPSx).unwrap();
            let mut upper: u16 = upp_hi as u16 * 256 + upp_lo as u16;
            loop {
                if upper - mid <= 1 {
                    break;
                }
                let new_mid = (upper + mid) / 2;
                tracing::debug!("SCPS neg: trying {} b", new_mid);
                ch.write_reg(
                    REG_SCPSx,
                    [0b1000_0000, 0, (new_mid / 256) as u8, (new_mid % 256) as u8],
                )
                .unwrap();
                let written = ch.read_reg(REG_SCPSx).unwrap();
                let actual = written[2] as u16 * 256 + written[3] as u16;
                match (got_packet_rx.recv().unwrap().is_some(), actual == new_mid) {
                    (true, true) => {
                        mid = new_mid;
                        largest = new_mid;
                        tracing::debug!("SCPS neg: success");
                    }
                    (true, false) => {
                        upper = new_mid;
                        tracing::debug!("SCPS neg: writing failure ({})", actual);
                    }
                    (false, _) => {
                        upper = new_mid;
                        tracing::debug!("SCPS neg: test packed missed");
                    }
                };
            }
            ch.write_reg(
                REG_SCPSx,
                [0, 0, (largest / 256) as u8, (largest % 256) as u8],
            )
            .unwrap();
            */
        });
    });
}

#[tokio::main]
async fn main() {
    let filter = tracing_subscriber::EnvFilter::from_default_env();
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let mut c = cameleon_device::gige::enumerate_devices(std::time::Duration::from_secs(1))
        .await
        .unwrap();

    println!("count: {}", c.len());
    let di = c.swap_remove(0);
    let mut ch = ControlHandle::new(di).unwrap();
    ch.open().unwrap();

    const IMG_PORT: u16 = 59998;

    for (port, txt) in [(IMG_PORT, "IMG"), (9999, "MSG")] {
        tokio::spawn(async move {
            let sock = UdpSocket::bind(("192.168.1.3", port)).unwrap();

            let mut acc_raw_data: Vec<u8> = Vec::new();
            let mut acc_block_id = u16::MAX;
            let mut acc_res = (0, 0);
            loop {
                let mut buf = [0u8; 10000];
                let size = sock.recv(&mut buf).unwrap();
                // tracing::debug!("{}: {:?}\n{} LEN: {}", txt, &buf[..size], txt, size);
                tracing::debug!("{} LEN: {}", txt, size);
                let cut = &buf[0..size];
                let mut cursor = Cursor::new(&cut);
                let status = cursor.read_u16().await.unwrap();
                let block_id = cursor.read_u16().await.unwrap();
                let ei_reserved_packet_format = cursor.read_u8().await.unwrap();
                let ei = (ei_reserved_packet_format & 0b1000_0000) > 0;
                let packet_format = ei_reserved_packet_format % 0b1000;
                let mut packet_id_buf = [0; 3];
                cursor.read_exact(&mut packet_id_buf).await.unwrap();
                let packet_id = packet_id_buf[0] as u32 * 0x1_00_00
                    + packet_id_buf[1] as u32 * 0x1_00
                    + packet_id_buf[2] as u32;
                if ei {
                    let _block_id64 = cursor.read_u64().await.unwrap();
                    let _packet_id32 = cursor.read_u32().await.unwrap();
                    println!(
                        "_block_id64: {} _packet_id32: {}",
                        _block_id64, _packet_id32
                    );
                }
                println!(
                    "Stat: {} B-ID: {} EI: {} Pack-Format: {} Pack-ID: {}",
                    status, block_id, ei, packet_format, packet_id
                );
                match packet_format {
                    // Leader
                    1 => {
                        println!("Packet type: LEADER");
                        let field_id_count = cursor.read_u16().await.unwrap();
                        let payload_type = cursor.read_u16().await.unwrap();
                        assert_eq!(payload_type, 1);
                        let timestamp = cursor.read_u64().await.unwrap();
                        let pixel_format = cursor.read_u32().await.unwrap();
                        let size_x = cursor.read_u32().await.unwrap();
                        let size_y = cursor.read_u32().await.unwrap();
                        let offset_x = cursor.read_u32().await.unwrap();
                        let offset_y = cursor.read_u32().await.unwrap();
                        let padding_x = cursor.read_u16().await.unwrap();
                        let padding_y = cursor.read_u16().await.unwrap();
                        // BayerRG8
                        println!("Pixel format: {:2X}", pixel_format);
                        println!(
                            "{}×{}+{}×{} (p: {}, {})",
                            size_x, size_y, offset_x, offset_y, padding_x, padding_y
                        );
                        assert!(cursor.remaining_slice().is_empty());
                        acc_raw_data.resize((size_x * size_y) as usize, 0);
                        acc_block_id = block_id;
                        acc_res = (size_x, size_y);
                    }
                    // Trailer
                    2 => {
                        println!("Packet type: TRAILER");
                        let (w, h) = acc_res;
                        let mut rgb = vec![0u8; acc_raw_data.len() * 3];
                        let mut raw = Cursor::new(&acc_raw_data);
                        let mut raster = bayer::RasterMut::new(
                            w as usize,
                            h as usize,
                            bayer::RasterDepth::Depth8,
                            &mut rgb,
                        );
                        bayer::demosaic(
                            &mut raw,
                            bayer::BayerDepth::Depth8,
                            bayer::CFA::RGGB,
                            bayer::Demosaic::Linear,
                            &mut raster,
                        )
                        .unwrap();
                        let buffer: ImageBuffer<Rgb<u8>, Vec<u8>> =
                            ImageBuffer::from_vec(w, h, rgb).unwrap();
                        buffer.save("./a.png").unwrap();
                    }
                    // Generic
                    3 => {
                        let data = cursor.remaining_slice();
                        println!("Packet type: GENERIC {}", data.len());
                        let target_len = acc_raw_data.len();
                        let target = &mut acc_raw_data[(8960 * (packet_id - 1)) as usize
                            ..((8960 * packet_id) as usize).min(target_len)];
                        target.copy_from_slice(data);
                    }

                    other => {
                        tracing::error!("Unrecognized packet format: {}", other);
                    }
                }
            }
        });
    }
    ch.write_reg(0x80028c64, [0, 0, 0, 2]).unwrap();
    println!("AcquisitionMode: {:#?}", ch.read_reg(0x80028c64).unwrap());
    println!("TriggerMode: {:#?}", ch.read_reg(0x80028c20).unwrap());

    // ch.write_reg(REG_MCDA, [192, 168, 1, 3]).unwrap();
    ch.write_reg(REG_MCDA, [0, 0, 0, 0]).unwrap();
    ch.write_reg(REG_SCDA, [192, 168, 1, 3]).unwrap();

    // ch.write_reg(REG_MCP, [0, 0, 0x27, 0x0F]).unwrap();
    ch.write_reg(REG_MCP, [0, 0, 0, 0]).unwrap();
    ch.write_reg(
        REG_SCP,
        [1, 0, (IMG_PORT / 256) as u8, (IMG_PORT % 256) as u8],
    )
    .unwrap();

    let size = 8996;
    ch.write_reg(
        REG_SCPS,
        [0b0100_0000, 0, (size / 256) as u8, (size % 256) as u8],
    )
    .unwrap();

    println!("SCP  = {:?}", ch.read_reg(REG_SCP).unwrap());
    println!("MCP  = {:?}", ch.read_reg(REG_MCP).unwrap());
    println!("SCPS = {:?}", ch.read_reg(REG_SCPS).unwrap());
    println!("SCSP = {:?}", ch.read_reg(REG_SCSP).unwrap());
    println!(
        "Number of Streams = {:?}",
        ch.read_reg(REG_NumOfStreams).unwrap()
    );
    println!(
        "Number of Messages = {:?}",
        ch.read_reg(REG_NumOfMessages).unwrap()
    );
    let [_, _, srcport_hi, srcport_lo] = ch.read_reg(REG_SCSP).unwrap();
    println!(
        "SRC UDP PORT: {}",
        srcport_hi as u16 * 256 + srcport_lo as u16
    );
    println!("SCDA = {:?}", ch.read_reg(REG_SCDA).unwrap());
    ch.write_reg(0x80028cb0, [0, 0, 0, 1]).unwrap();

    thread::sleep(std::time::Duration::from_secs(1));

    ch.write_reg(0x80028cb4, [0, 0, 0, 1]).unwrap();

    println!("DONE");
    ch.write_reg(REG_SCP, [0, 0, 0, 0]).unwrap();
    ch.write_reg(REG_MCP, [0, 0, 0, 0]).unwrap();
    // ch.enable_streaming().unwrap();
    // ch.disable_streaming().unwrap();
    ch.close().unwrap();

    //
    //
    //
    //
    //
    // let mut c = enumerate_cameras().unwrap();
    // let mut c = c.swap_remove(0);
    // let mut ctx = c.params_ctxt().unwrap();
    // let node = ctx.node("AcquisitionMode").unwrap();
    // node.name(ctxt)
    // println!("{:#?}", node);
    // println!("{}", ctx);
    // c.open().unwrap();
    // let rx = c.start_streaming(1).unwrap();
    // let res = rx.recv().await.unwrap();
    // println!("{:?}", res.image_info());
    // c.close().unwrap();
    // let socket = UdpSocket::bind("192.168.1.3:0").unwrap();
    // socket.set_broadcast(true).unwrap();
    // socket.send_to(&[0x42, ], "255.255.255.255:0").unwrap();
}
