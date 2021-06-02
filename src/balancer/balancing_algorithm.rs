use super::algorithms::RoundRobin;

trait BalancingAlgorithm {
    fn get_next_host() -> String;
}

impl BalancingAlgorithm for RoundRobin {
    fn get_next_host() -> String {
        "".to_string()
    }
}