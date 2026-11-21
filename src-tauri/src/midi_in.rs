//! Hardware MIDI input: connects to every available input port and forwards
//! note and controller messages (with their channel) to a callback.

use midir::{Ignore, MidiInput, MidiInputConnection};
use std::sync::Arc;

/// Channels are 1..16.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidiMsg {
    NoteOn { channel: u8, pitch: u8, velocity: u8 },
    NoteOff { channel: u8, pitch: u8 },
    Control { channel: u8, cc: u8, value: u8 },
}

pub fn parse(msg: &[u8]) -> Option<MidiMsg> {
    let status = *msg.first()?;
    let channel = (status & 0x0F) + 1;
    match status & 0xF0 {
        0x90 if msg.len() >= 3 => {
            if msg[2] == 0 {
                Some(MidiMsg::NoteOff { channel, pitch: msg[1] })
            } else {
                Some(MidiMsg::NoteOn { channel, pitch: msg[1], velocity: msg[2] })
            }
        }
        0x80 if msg.len() >= 3 => Some(MidiMsg::NoteOff { channel, pitch: msg[1] }),
        0xB0 if msg.len() >= 3 => Some(MidiMsg::Control { channel, cc: msg[1], value: msg[2] }),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_channel() {
        assert_eq!(parse(&[0x99, 36, 100]), Some(MidiMsg::NoteOn { channel: 10, pitch: 36, velocity: 100 }));
        assert_eq!(parse(&[0x90, 60, 0]), Some(MidiMsg::NoteOff { channel: 1, pitch: 60 }));
        assert_eq!(parse(&[0xB3, 74, 64]), Some(MidiMsg::Control { channel: 4, cc: 74, value: 64 }));
    }
}
