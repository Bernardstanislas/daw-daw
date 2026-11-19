//! Hardware MIDI input: connects to every available input port and forwards
//! note messages to a callback.

use midir::{Ignore, MidiInput, MidiInputConnection};
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub enum MidiMsg {
    NoteOn { pitch: u8, velocity: u8 },
    NoteOff { pitch: u8 },
    /// Sustain pedal etc. (not routed yet, kept for later).
    Control { cc: u8, value: u8 },
}

pub fn parse(msg: &[u8]) -> Option<MidiMsg> {
    let status = *msg.first()?;
    match status & 0xF0 {
        0x90 if msg.len() >= 3 => {
            if msg[2] == 0 {
                Some(MidiMsg::NoteOff { pitch: msg[1] })
            } else {
                Some(MidiMsg::NoteOn { pitch: msg[1], velocity: msg[2] })
            }
        }
        0x80 if msg.len() >= 3 => Some(MidiMsg::NoteOff { pitch: msg[1] }),
        0xB0 if msg.len() >= 3 => Some(MidiMsg::Control { cc: msg[1], value: msg[2] }),
        _ => None,
    }
}

pub struct MidiInputs {
    connections: Vec<MidiInputConnection<()>>,
    pub port_names: Vec<String>,
}

impl MidiInputs {
    pub fn connect_all(callback: Arc<dyn Fn(MidiMsg) + Send + Sync>) -> Self {
        let mut connections = Vec::new();
        let mut port_names = Vec::new();
        let Ok(probe) = MidiInput::new("daw-daw-probe") else {
            return Self { connections, port_names };
        };
        for port in probe.ports() {
            let name = probe.port_name(&port).unwrap_or_else(|_| "MIDI".into());
            let Ok(mut input) = MidiInput::new("daw-daw") else { continue };
            input.ignore(Ignore::All);
            let cb = callback.clone();
            match input.connect(
                &port,
                "daw-daw-in",
                move |_stamp, message, _| {
                    if let Some(m) = parse(message) {
                        cb(m);
                    }
                },
                (),
            ) {
                Ok(conn) => {
                    connections.push(conn);
                    port_names.push(name);
                }
                Err(e) => eprintln!("[midi] cannot connect {name}: {e}"),
            }
        }
        Self { connections, port_names }
    }

    pub fn close(self) {
        for c in self.connections {
            c.close();
        }
    }
}
