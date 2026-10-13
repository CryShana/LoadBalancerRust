use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use super::BalancingAlgorithm;

pub struct RoundRobin {
    hosts: Vec<SocketAddr>,
    next: usize,
    cooldowns: HashMap<SocketAddr, Instant>,
}

impl RoundRobin {
    // how long the host is avoided (on cooldown) after an error is reported
    const TARGET_DOWN_COOLDOWN: Duration = Duration::from_secs(30);

    pub fn new(hosts: Vec<SocketAddr>) -> Self {
        assert!(!hosts.is_empty(), "RoundRobin requires at least one host");

        RoundRobin {
            hosts,
            next: 0,
            cooldowns: HashMap::new(),
        }
    }

    fn advance(&mut self) -> SocketAddr {
        let host = self.hosts[self.next];
        self.next = (self.next + 1) % self.hosts.len();
        host
    }
}

impl BalancingAlgorithm for RoundRobin {
    fn get_next_host(&mut self) -> SocketAddr {
        let now = Instant::now();

        for _ in 0..self.hosts.len() {
            let host = self.advance();
            if self.cooldowns.get(&host).is_none_or(|&until| now >= until) {
                return host;
            }
        }

        // every host is on cooldown, keep rotating through them until one connects again
        self.advance()
    }

    fn report_error(&mut self, addr: SocketAddr) {
        self.cooldowns.insert(addr, Instant::now() + Self::TARGET_DOWN_COOLDOWN);
    }

    fn report_success(&mut self, addr: SocketAddr) {
        self.cooldowns.remove(&addr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts() -> Vec<SocketAddr> {
        vec!["127.0.0.1:1".parse().unwrap(), "127.0.0.1:2".parse().unwrap(), "127.0.0.1:3".parse().unwrap()]
    }

    #[test]
    fn rotates_through_hosts() {
        let hosts = hosts();
        let mut rr = RoundRobin::new(hosts.clone());

        let picked: Vec<_> = (0..4).map(|_| rr.get_next_host()).collect();
        assert_eq!(picked, [hosts[0], hosts[1], hosts[2], hosts[0]]);
    }

    #[test]
    fn skips_hosts_on_cooldown_until_success() {
        let hosts = hosts();
        let mut rr = RoundRobin::new(hosts.clone());
        rr.report_error(hosts[1]);

        let picked: Vec<_> = (0..3).map(|_| rr.get_next_host()).collect();
        assert_eq!(picked, [hosts[0], hosts[2], hosts[0]]);

        rr.report_success(hosts[1]);
        assert_eq!(rr.get_next_host(), hosts[1]);
    }

    #[test]
    fn rotates_when_all_hosts_on_cooldown() {
        let hosts = hosts();
        let mut rr = RoundRobin::new(hosts.clone());
        hosts.iter().for_each(|&h| rr.report_error(h));

        let picked: Vec<_> = (0..3).map(|_| rr.get_next_host()).collect();
        assert_eq!(picked, [hosts[0], hosts[1], hosts[2]]);
    }
}
