use std::net::SocketAddr;
pub trait BalancingAlgorithm: Sync + Send {
    /**
        Returns the next host for the client to try to connect to    
    */
    fn get_next_host(&mut self) -> SocketAddr;
    /**
        Reports error for the given host address. Host can then be placed on cooldown, this can affect the [get_next_host] call
    */
    fn report_error(&mut self, addr: SocketAddr);
    /**
        Reports success for the given host address. Host can be removed from cooldown
    */
    fn report_success(&mut self, addr: SocketAddr);
    /**
        Checks if host is currently on cooldown or in any way affected by the reported errors
    */
    fn is_on_cooldown(&self, addr: SocketAddr) -> bool;
}
