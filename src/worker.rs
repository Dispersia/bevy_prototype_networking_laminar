use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use bytes::Bytes;

use laminar::{Packet, Socket, SocketEvent};

use super::{Connection, Message, NetworkEvent, NetworkResource, SocketHandle, SocketInstruction};

pub fn start_worker_thread() -> NetworkResource {
    let (event_tx, event_rx): (Sender<NetworkEvent>, Receiver<NetworkEvent>) = mpsc::channel();
    let (message_tx, message_rx): (Sender<Message>, Receiver<Message>) = mpsc::channel();
    let (instruction_tx, instruction_rx): (Sender<SocketInstruction>, Receiver<SocketInstruction>) =
        mpsc::channel();

    let mut sockets = TrackedSockets {
        sockets: Vec::new(),
    };

    let sleep_time = Duration::from_millis(1);

    let resource = NetworkResource {
        default_socket: None,
        connections: Vec::new(),
        message_tx: Mutex::new(message_tx),
        event_rx: Mutex::new(event_rx),
        instruction_tx: Mutex::new(instruction_tx),
    };

    let mut start = std::time::Instant::now();

    thread::spawn(move || loop {
        let millis = start.elapsed().as_millis();
        if millis > 50 {
            println!("worker elapsed: {:?}", start.elapsed());
        }
        start = std::time::Instant::now();

        handle_instructions(&mut sockets, &instruction_rx);
        poll_sockets(&mut sockets);
        send_messages(&mut sockets, &message_rx);
        receive_messages(&mut sockets, &event_tx);

        // go dark
        std::thread::sleep(sleep_time);
    });

    return resource;
}

fn handle_instructions(sockets: &mut TrackedSockets, instruction_rx: &Receiver<SocketInstruction>) {
    while let Ok(instruction) = instruction_rx.try_recv() {
        match instruction {
            SocketInstruction::AddSocket(handle, socket) => {
                sockets.add_socket(handle, socket);
            } // future work: allow manual closing of sockets
              // SocketInstruction::CloseSocket(handle) => {
              //     sockets.close_socket(handle);
              // }
        }
    }
}

fn poll_sockets(sockets: &mut TrackedSockets) {
    for (_, socket) in sockets.iter_mut() {
        socket.manual_poll(Instant::now());
    }
}

fn send_messages(sockets: &mut TrackedSockets, message_rx: &Receiver<Message>) {
    while let Ok(message) = message_rx.try_recv() {
        let handle = message.socket_handle;
        let sopt = sockets.get_socket_mut(handle);
        match sopt {
            Some(socket) => {
                socket
                    .send(Packet::reliable_unordered(
                        message.destination,
                        message.message.to_vec(),
                    ))
                    .expect("This should send");
            }
            None => {
                println!("Attempted to send a message to a handle that is not open. Ignoring the message");
            }
        }
    }
}

fn receive_messages(sockets: &mut TrackedSockets, event_tx: &Sender<NetworkEvent>) {
    for (socket_handle, socket) in sockets.iter_mut() {
        while let Some(event) = socket.recv() {
            let e = match event {
                SocketEvent::Connect(addr) => Some(NetworkEvent::Connected(Connection {
                    addr: addr,
                    socket: *socket_handle,
                })),
                SocketEvent::Timeout(addr) => Some(NetworkEvent::Disconnected(Connection {
                    addr: addr,
                    socket: *socket_handle,
                })),
                SocketEvent::Packet(packet) => Some(NetworkEvent::Message(
                    Connection {
                        addr: packet.addr(),
                        socket: *socket_handle,
                    },
                    Bytes::copy_from_slice(packet.payload()),
                )),
            };

            if let Some(e) = e {
                event_tx.send(e).unwrap();
            }
        }
    }
}

struct TrackedSockets {
    sockets: Vec<(SocketHandle, Socket)>,
}

impl TrackedSockets {
    pub fn iter_mut(&mut self) -> std::slice::IterMut<(SocketHandle, Socket)> {
        return self.sockets.iter_mut();
    }

    pub fn add_socket(&mut self, handle: SocketHandle, socket: Socket) {
        if self.has_socket(handle) {
            // todo: communicate socket error back
            println!(
                "Warning: attempted to add socket with an existing handle, dropping the new socket"
            );
            return;
        }

        return self.sockets.push((handle, socket));
    }

    // pub fn close_socket(&mut self, handle: SocketHandle) {
    //     let sock = self.sockets.iter().position(|(h, _)| *h == handle);

    //     match sock {
    //         Some(idx) => {
    //             self.sockets.remove(idx);
    //         }
    //         None => {
    //             println!("Warning: attempting to close a socket that doesn't exist.");
    //         }
    //     }
    // }

    pub fn has_socket(&self, handle: SocketHandle) -> bool {
        return match self.get_socket(handle) {
            Some(_) => true,
            None => false,
        };
    }

    pub fn get_socket(&self, handle: SocketHandle) -> Option<&Socket> {
        return self
            .sockets
            .iter()
            .find(|(h, _)| handle == *h)
            .and_then(|(_, s)| Some(s));
    }

    pub fn get_socket_mut(&mut self, handle: SocketHandle) -> Option<&mut Socket> {
        return self
            .sockets
            .iter_mut()
            .find(|(h, _)| handle == *h)
            .and_then(|(_, s)| Some(s));
    }
}