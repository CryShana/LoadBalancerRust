use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::usize;
use std::{
    net::{IpAddr, SocketAddr, TcpStream},
    thread,
    time::Duration,
    u16,
};

use super::BalancingAlgorithm;
use super::RoundRobin;
use super::TcpClient;
const CONNECTION_TIMEOUT: Duration = Duration::from_millis(1500);
const SLEEP_TIME: Duration = Duration::from_millis(5);

pub struct LoadBalancer {
    clients: Arc<RwLock<Vec<Arc<RwLock<TcpClient>>>>>,
    stopped: Arc<RwLock<bool>>,
    debug: Arc<RwLock<bool>>,
    threads: u16,
    balancing_algorithm: Arc<Mutex<RoundRobin>>,
}

impl LoadBalancer {
    pub fn new(balancing_algorithm: RoundRobin, threads: u16, debug: bool) -> Self {
        let mut b = LoadBalancer {
            clients: Arc::new(RwLock::new(vec![])),
            stopped: Arc::new(RwLock::new(false)),
            debug: Arc::new(RwLock::new(debug)),
            threads,
            balancing_algorithm: Arc::new(Mutex::new(balancing_algorithm)),
        };

        b.spawn_workers();

        b
    }

    pub fn add_client(&mut self, stream: TcpStream) {
        let client = TcpClient::new(stream);
        self.clients
            .write()
            .unwrap()
            .push(Arc::new(RwLock::new(client)));
    }

    pub fn stop(&mut self) {
        *self.stopped.write().unwrap() = true;
    }

    fn spawn_workers(&mut self) {
        let th = self.threads as u32;

        for id in 0..th {
            let c = Arc::clone(&self.clients);
            let s = Arc::clone(&self.stopped);
            let d = Arc::clone(&self.debug);
            let b = Arc::clone(&self.balancing_algorithm);

            // SPAWN PROCESSORS
            thread::spawn(move || loop {
                thread::sleep(SLEEP_TIME);
                
                // HANDLE CLIENTS
                {
                    let clients = &*c.read().unwrap();
                    let length = clients.len() as u32;
                    let mut capacity = length / th;

                    // if there are less clients than threads, we can let the first thread handle all of them
                    if length < th {
                        if id == 0 {
                            // first thread will take on all of them
                            capacity = length;
                        } else {
                            // other threads are ignored for now
                            continue;
                        }
                    }

                    // every thread starts at a specified index and handles [capacity] clients
                    let starting_index = id * capacity;
                    let mut end_index = starting_index + capacity; // exclusive
                    if id == th - 1 {
                        // if this is the last thread, just handle all of rest
                        end_index = length;
                    }

                    // handle clients
                    for i in starting_index..end_index {
                        let mut client = match clients.get(i as usize) {
                            Some(client) => client.write().unwrap(),
                            None => {
                                if *d.read().unwrap() {
                                    println!(
                                        "[Thread {}] Cancelled early because collection changed",
                                        id
                                    );
                                }
                                break;
                            }
                        };

                        // ignore clients that are no longer connected
                        if client.is_client_connected() == false {
                            continue;
                        }

                        // handle client
                        if client.is_connected() {
                            let success = client.process();
                            if success == false {
                                // connection to either server or client has failed

                                // removal from list is handled later
                                if *d.read().unwrap() {
                                    println!(
                                        "[Thread {}] Connection ended ({})",
                                        id, client.address
                                    );
                                }
                            }
                        } else {
                            // determine target host to connect to, using the balancing algorithm!
                            let target_socket = b.lock().unwrap().get_next_host();

                            if *d.read().unwrap() {
                                println!(
                                    "[Thread {}] Connecting client ({} -> {})",
                                    id, client.address, target_socket
                                );
                            }

                            // connect to host!
                            // TODO: this blocks! solve it differently!
                            let success =
                                client.connect_to_target(target_socket, CONNECTION_TIMEOUT);

                            if *d.read().unwrap() {
                                if success {
                                    println!(
                                        "[Thread {}] Connected client ({} -> {})",
                                        id, client.address, target_socket
                                    );
                                } else {
                                    println!(
                                        "[Thread {}] Failed to connect client ({} -> {})",
                                        id, client.address, target_socket
                                    );
                                }
                            }
                        }
                    }
                }

                // keep checking if balancer has been stopped
                let stopped = *s.read().unwrap();
                if stopped == true {
                    break;
                }
            });
        }

        // SPAWN CLEANER - will clean disconnected clients from vector
        let c = Arc::clone(&self.clients);
        let s = Arc::clone(&self.stopped);
        thread::spawn(move || loop {
            loop {
                thread::sleep(Duration::from_secs(5));

                let mut clients = c.write().unwrap();
                let mut len = clients.len();
                let mut i = 0;

                loop {
                    if i >= len {
                        break;
                    }

                    if clients[i].read().unwrap().is_client_connected() == false {
                        clients.remove(i);
                        len = len - 1;
                        continue;
                    }

                    i = i + 1;
                }

                let stopped = *s.read().unwrap();
                if stopped == true {
                    break;
                }
            }
        });
    }
}
