use std::net::SocketAddr;
use std::time::Duration;
use std::time::Instant;
use std::usize;

use super::BalancingAlgorithm;
use super::HostManager;

pub struct RoundRobin {
    current_host: usize,
    max_host: usize,
    host_manager: HostManager,
    cooldowns: Vec<(SocketAddr, Instant)>,
}

impl RoundRobin {
    // how long the host is avoided (on cooldown) when first error is reported
    const TARGET_DOWN_COOLDOWN: Duration = Duration::from_secs(10);

    pub fn new(host_manager: HostManager) -> Self {
        let max = host_manager.hosts.len();
        RoundRobin {
            current_host: 0,
            host_manager: host_manager,
            max_host: max,
            cooldowns: vec![],
        }
    }

    fn get_host_cooldown_index(&self, addr: SocketAddr) -> i32 {
        let mut index: i32 = -1;
        for i in 0..self.cooldowns.len() {
            if self.cooldowns[i].0 == addr {
                index = i as i32;
                break;
            }
        }

        index
    }
}

impl BalancingAlgorithm for RoundRobin {
    fn get_next_host(&mut self) -> SocketAddr {
        let mut val;
        let starting_host_index = self.current_host;

        loop {
            // select host
            val = self.host_manager.hosts[self.current_host];

            // offset host selector to next one
            self.current_host = self.current_host + 1;
            if self.current_host >= self.max_host {
                self.current_host = 0
            }

            // if host on cooldown, avoid it (but if we made a full cycle, just return the initial choice)
            let cooldown_index = self.get_host_cooldown_index(val);
            if cooldown_index >= 0 && starting_host_index != self.current_host {
                // check if cooldown has passed
                if Instant::now() > self.cooldowns[cooldown_index as usize].1 {
                    // cooldown passed, remove it
                    self.cooldowns.remove(cooldown_index as usize);
                    break;
                }

                continue;
            }

            break;
        }

        val
    }

    fn report_error(&mut self, addr: SocketAddr) {
        // find index of cooldown if it exists
        let index: i32 = self.get_host_cooldown_index(addr);

        let new_limit = Instant::now() + RoundRobin::TARGET_DOWN_COOLDOWN;

        if index < 0 {
            // add it
            self.cooldowns.push((addr, new_limit));
        } else {
            // update it
            self.cooldowns[index as usize].1 = new_limit;
        }
    }
}
