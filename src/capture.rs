//! A capture thread per network adapter, all feeding one thread that decodes.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

use crate::net::{self, StreamKey, Streams};
use crate::npcap::{Capture, Next, Npcap};
use crate::overlay::{epoch_ms, Overlay};
use crate::packets::{self, Event};
use crate::tracker::Tracker;

const FILTER: &str = "tcp";
const TRACE_EVERY: Duration = Duration::from_secs(15);
const QUEUE: usize = 65_536;
/// Virtual adapters that never carry the game
const SKIP: [&str; 6] = ["WAN Miniport", "Hyper-V", "VMware", "VirtualBox", "Bluetooth", "Wi-Fi Direct"];

type Frame = (i32, Vec<u8>);

/// `trace` logs every cast and how many of each packet arrived, for working out
/// what a patch changed
pub fn start(overlay: Overlay, replay: Option<String>, trace: bool) {
    let npcap = match Npcap::load() {
        Ok(npcap) => npcap,
        Err(e) => return overlay.error(e),
    };
    let (tx, rx) = mpsc::sync_channel::<Frame>(QUEUE);
    if let Some(path) = replay {
        match npcap.open_file(&path, FILTER) {
            Ok(capture) => {
                overlay.log(format!("replaying {path}"));
                read(capture, tx, true, overlay.clone());
            }
            Err(e) => return overlay.error(format!("Can't open {path}: {e}")),
        }
    } else {
        let adapters = match npcap.adapters() {
            Ok(adapters) => adapters,
            Err(e) => return overlay.error(format!("Npcap: {e}")),
        };
        let mut opened = 0;
        for adapter in adapters.iter().filter(|a| !SKIP.iter().any(|s| a.description.contains(s))) {
            match npcap.open(&adapter.name, FILTER) {
                Ok(capture) => {
                    overlay.log(format!("capturing on {} ({})", adapter.description, adapter.name));
                    read(capture, tx.clone(), false, overlay.clone());
                    opened += 1;
                }
                Err(e) => overlay.log(format!("skipped {} ({}): {e}", adapter.description, adapter.name)),
            }
        }
        if opened == 0 {
            return overlay.error("Npcap can't open a network adapter");
        }
    }
    thread::spawn(move || decode(rx, overlay, trace));
}

fn read(mut capture: Capture, tx: SyncSender<Frame>, replay: bool, overlay: Overlay) {
    thread::spawn(move || {
        let link = capture.link;
        loop {
            match capture.next() {
                Ok(Next::Packet(data)) => {
                    let frame = (link, data.to_vec());
                    // live capture never waits on the decoder: a full queue drops the frame
                    let sent = if replay {
                        tx.send(frame).is_ok()
                    } else {
                        !matches!(tx.try_send(frame), Err(TrySendError::Disconnected(_)))
                    };
                    if !sent {
                        break;
                    }
                }
                Ok(Next::Timeout) => {}
                Ok(Next::End) => {
                    overlay.log("replay finished");
                    break;
                }
                Err(e) => {
                    overlay.log(format!("capture stopped: {e}"));
                    break;
                }
            }
        }
    });
}

fn decode(rx: Receiver<Frame>, overlay: Overlay, trace: bool) {
    let mut streams = Streams::new();
    let mut tracker = Tracker::default();
    let mut game: HashSet<StreamKey> = HashSet::new();
    let mut events = Vec::new();
    let mut counts: HashMap<u16, u32> = HashMap::new();
    let mut counted = Instant::now();
    for (link, frame) in rx {
        let Some(segment) = net::segment(link, &frame) else { continue };
        streams.push(segment, Instant::now(), &mut |key, opcode, body| {
            if game.insert(*key) {
                overlay.log(format!("game traffic: {key}"));
                overlay.update(|v| v.traffic = true);
            }
            if trace {
                *counts.entry(opcode).or_default() += 1;
            }
            events.extend(packets::parse(opcode, body).or_else(|| packets::embedded_self(body)));
        });
        for event in events.drain(..) {
            match &event {
                Event::SelfInfo { entity, .. } => overlay.log(format!("our entity: {entity}")),
                Event::Cast { .. } | Event::Cooldowns(_) if trace => overlay.log(format!("{event:?}")),
                _ => {}
            }
            for update in tracker.handle(event, epoch_ms()) {
                overlay.apply(update);
            }
        }
        if trace && counted.elapsed() > TRACE_EVERY && !counts.is_empty() {
            let mut top: Vec<_> = counts.drain().collect();
            top.sort_by(|a, b| b.1.cmp(&a.1));
            let top: Vec<String> = top.iter().take(24).map(|(op, n)| format!("{:02x} {:02x}: {n}", op >> 8, op & 0xFF)).collect();
            overlay.log(format!("packets: {}", top.join(", ")));
            counted = Instant::now();
        }
    }
}
