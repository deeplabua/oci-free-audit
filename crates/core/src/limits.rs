//! Oracle's Always Free numbers — the only place they live.
//!
//! Oracle changes these without notice (15 June 2026: A1 went from 4 OCPU / 24 GB to
//! 2 OCPU / 12 GB). When one changes: update it here, bump [`CHECKED`], add a CHANGELOG line.

/// When the numbers below were last checked against [`SOURCE`].
pub const CHECKED: &str = "2026-11-10";

/// Oracle's list of Always Free resources.
pub const SOURCE: &str = "https://docs.oracle.com/iaas/Content/FreeTier/resourceref.htm";

/// The Ampere A1 Flex shape.
pub const A1_FLEX: &str = "VM.Standard.A1.Flex";

/// The AMD micro shape.
pub const E2_MICRO: &str = "VM.Standard.E2.1.Micro";

/// A1 OCPUs and memory (GB) that are free on every account since 15 June 2026.
pub const A1_FREE: Allowance = Allowance {
    ocpus: 2.0,
    memory_gb: 12.0,
};

/// The pre-June-2026 A1 allowance. Some pay-as-you-go tenancies still pay nothing for it
/// (measured 10 November 2026 on one created 4 June 2026); above it is never free.
pub const A1_LEGACY: Allowance = Allowance {
    ocpus: 4.0,
    memory_gb: 24.0,
};

/// E2.1.Micro instances that are free.
pub const E2_MICRO_MAX: usize = 2;

/// Boot and block volumes together, GB.
pub const BLOCK_STORAGE_GB: f64 = 200.0;

/// Boot and block volume backups together.
pub const VOLUME_BACKUPS: usize = 5;

/// Outbound data transfer per month, GB (10 TB).
pub const EGRESS_GB_PER_MONTH: f64 = 10_240.0;

/// Object Storage on an account with only Always Free: all tiers together, GB.
pub const OBJECT_STORAGE_GB: f64 = 20.0;

/// Object Storage on a pay-as-you-go account: each of Standard and Archive, GB.
pub const OBJECT_STORAGE_PAYG_GB_PER_TIER: f64 = 10.0;

/// Always Free Autonomous Databases.
pub const AUTONOMOUS_DB_MAX: usize = 2;

/// Always Free MySQL HeatWave DB systems.
pub const MYSQL_MAX: usize = 1;

/// The Always Free MySQL HeatWave shape.
pub const MYSQL_FREE_SHAPE: &str = "MySQL.Free";

/// Always Free flexible load balancers.
pub const LB_MAX: usize = 1;

/// Bandwidth of the Always Free flexible load balancer, Mbit/s.
pub const LB_FREE_MBPS: f64 = 10.0;

/// Always Free network load balancers.
pub const NLB_MAX: usize = 1;

/// VCNs on an account with only Always Free.
pub const VCN_MAX: usize = 2;

/// Usage API SKU names that are A1 compute (OCPU and memory).
pub const A1_SKUS: [&str; 2] = ["Standard - A1", "Standard - A1 - Memory"];

/// Usage API SKU names that count outbound data transfer, matched by prefix.
pub const EGRESS_SKU_PREFIX: &str = "Outbound Data Transfer";

/// OCPUs and memory.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Allowance {
    /// OCPUs.
    pub ocpus: f64,
    /// Memory, GB.
    pub memory_gb: f64,
}

impl Allowance {
    /// `true` if `other` fits inside this allowance.
    pub fn covers(&self, other: Allowance) -> bool {
        other.ocpus <= self.ocpus + 1e-9 && other.memory_gb <= self.memory_gb + 1e-9
    }

    /// Parse `"4/24"` (OCPUs / GB).
    pub fn parse(text: &str) -> Option<Self> {
        let (ocpus, memory) = text.split_once('/')?;
        Some(Self {
            ocpus: ocpus.trim().parse().ok()?,
            memory_gb: memory
                .trim()
                .trim_end_matches(['G', 'g', 'B', 'b'])
                .trim()
                .parse()
                .ok()?,
        })
    }
}

impl std::fmt::Display for Allowance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} OCPU / {} GB", self.ocpus, self.memory_gb)
    }
}

/// Shapes that can be free at all.
pub fn is_free_shape(shape: &str) -> bool {
    shape == A1_FLEX || shape == E2_MICRO
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_compares_allowances() {
        let a = Allowance::parse("4/24").unwrap();
        assert_eq!(a, A1_LEGACY);
        assert_eq!(Allowance::parse(" 2 / 12GB ").unwrap(), A1_FREE);
        assert!(Allowance::parse("4").is_none());
        assert!(A1_LEGACY.covers(A1_FREE));
        assert!(!A1_FREE.covers(A1_LEGACY));
        assert_eq!(A1_FREE.to_string(), "2 OCPU / 12 GB");
    }

    #[test]
    fn only_two_shapes_are_free() {
        assert!(is_free_shape(A1_FLEX));
        assert!(is_free_shape(E2_MICRO));
        assert!(!is_free_shape("VM.Standard.E4.Flex"));
    }
}
