use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReconnectStrategy {
    Simple,
    Persistent,
    Configurable,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReconnectConfig {
    pub strategy: ReconnectStrategy,
    pub max_retries: Option<usize>,
    pub backoff_base_ms: u64,
    pub backoff_cap_ms: u64,
}

impl ReconnectConfig {
    pub fn simple() -> Self {
        Self {
            strategy: ReconnectStrategy::Simple,
            max_retries: Some(3),
            backoff_base_ms: 1_000,
            backoff_cap_ms: 30_000,
        }
    }

    pub fn persistent() -> Self {
        Self {
            strategy: ReconnectStrategy::Persistent,
            max_retries: None,
            backoff_base_ms: 1_000,
            backoff_cap_ms: 30_000,
        }
    }

    pub fn should_retry(&self, attempt: usize) -> bool {
        match self.max_retries {
            None => true,
            Some(max) => attempt < max,
        }
    }

    pub fn backoff_ms(&self, attempt: usize) -> u64 {
        // base * 2^attempt, capped at backoff_cap_ms
        let shift = attempt.min(63) as u32;
        let multiplier = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
        let uncapped = self.backoff_base_ms.saturating_mul(multiplier);
        uncapped.min(self.backoff_cap_ms)
    }
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self::simple()
    }
}

pub fn is_permanent_error(status: u16) -> bool {
    // 4xx errors are permanent except 429 (Too Many Requests)
    (400..500).contains(&status) && status != 429
}

// Custom Deserialize: accepts either a string shorthand or a full struct.
impl<'de> Deserialize<'de> for ReconnectConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ReconnectConfigVisitor)
    }
}

struct ReconnectConfigVisitor;

impl<'de> Visitor<'de> for ReconnectConfigVisitor {
    type Value = ReconnectConfig;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(r#"a string ("simple" or "persistent") or a reconnect config struct"#)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<ReconnectConfig, E> {
        match v {
            "simple" => Ok(ReconnectConfig::simple()),
            "persistent" => Ok(ReconnectConfig::persistent()),
            other => Err(E::unknown_variant(other, &["simple", "persistent"])),
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ReconnectConfig, A::Error> {
        let mut strategy: Option<ReconnectStrategy> = None;
        let mut max_retries: Option<Option<usize>> = None;
        let mut backoff_base_ms: Option<u64> = None;
        let mut backoff_cap_ms: Option<u64> = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "strategy" => {
                    strategy = Some(map.next_value()?);
                }
                "max_retries" => {
                    max_retries = Some(map.next_value()?);
                }
                "backoff_base_ms" => {
                    backoff_base_ms = Some(map.next_value()?);
                }
                "backoff_cap_ms" => {
                    backoff_cap_ms = Some(map.next_value()?);
                }
                unknown => {
                    return Err(de::Error::unknown_field(
                        unknown,
                        &[
                            "strategy",
                            "max_retries",
                            "backoff_base_ms",
                            "backoff_cap_ms",
                        ],
                    ));
                }
            }
        }

        let strategy = strategy.ok_or_else(|| de::Error::missing_field("strategy"))?;

        // Derive defaults from strategy when fields are absent.
        let base = match &strategy {
            ReconnectStrategy::Simple => ReconnectConfig::simple(),
            ReconnectStrategy::Persistent => ReconnectConfig::persistent(),
            ReconnectStrategy::Configurable => ReconnectConfig::simple(),
        };

        Ok(ReconnectConfig {
            strategy,
            max_retries: max_retries.unwrap_or(base.max_retries),
            backoff_base_ms: backoff_base_ms.unwrap_or(base.backoff_base_ms),
            backoff_cap_ms: backoff_cap_ms.unwrap_or(base.backoff_cap_ms),
        })
    }
}

// ReconnectStrategy needs Deserialize for the map visitor above.
impl<'de> Deserialize<'de> for ReconnectStrategy {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "simple" => Ok(ReconnectStrategy::Simple),
            "persistent" => Ok(ReconnectStrategy::Persistent),
            "configurable" => Ok(ReconnectStrategy::Configurable),
            other => Err(de::Error::unknown_variant(
                other,
                &["simple", "persistent", "configurable"],
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_defaults() {
        let config = ReconnectConfig::simple();
        assert_eq!(config.max_retries, Some(3));
        assert_eq!(config.backoff_base_ms, 1000);
        assert_eq!(config.backoff_cap_ms, 30_000);
    }

    #[test]
    fn persistent_no_max() {
        let config = ReconnectConfig::persistent();
        assert!(config.max_retries.is_none());
    }

    #[test]
    fn backoff_exponential_with_cap() {
        let config = ReconnectConfig::simple();
        assert_eq!(config.backoff_ms(0), 1000);
        assert_eq!(config.backoff_ms(1), 2000);
        assert_eq!(config.backoff_ms(2), 4000);
        assert_eq!(config.backoff_ms(10), 30_000); // capped
    }

    #[test]
    fn should_retry_simple() {
        let config = ReconnectConfig::simple();
        assert!(config.should_retry(0));
        assert!(config.should_retry(2));
        assert!(!config.should_retry(3));
    }

    #[test]
    fn should_retry_persistent() {
        let config = ReconnectConfig::persistent();
        assert!(config.should_retry(0));
        assert!(config.should_retry(100));
        assert!(config.should_retry(10_000));
    }

    #[test]
    fn permanent_error_detection() {
        assert!(is_permanent_error(401));
        assert!(is_permanent_error(403));
        assert!(is_permanent_error(404));
        assert!(!is_permanent_error(500));
        assert!(!is_permanent_error(502));
        assert!(!is_permanent_error(503));
        assert!(!is_permanent_error(429)); // rate limit is retryable
    }

    #[test]
    fn deserialize_string_shorthand() {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            reconnect: ReconnectConfig,
        }
        let w: Wrapper = toml::from_str(r#"reconnect = "simple""#).unwrap();
        assert_eq!(w.reconnect.strategy, ReconnectStrategy::Simple);
        assert_eq!(w.reconnect.max_retries, Some(3));
    }

    #[test]
    fn deserialize_struct_form() {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            reconnect: ReconnectConfig,
        }
        let toml_str = r#"
            [reconnect]
            strategy = "configurable"
            max_retries = 5
            backoff_base_ms = 2000
            backoff_cap_ms = 60000
        "#;
        let w: Wrapper = toml::from_str(toml_str).unwrap();
        assert_eq!(w.reconnect.strategy, ReconnectStrategy::Configurable);
        assert_eq!(w.reconnect.max_retries, Some(5));
        assert_eq!(w.reconnect.backoff_base_ms, 2000);
        assert_eq!(w.reconnect.backoff_cap_ms, 60_000);
    }
}
