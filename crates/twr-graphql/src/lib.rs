//! `twr-graphql` — query-ID resolution + `endpoints.yaml` + features.
//!
//! Four resolution layers per op (plan §7): `TWR_QID_<OP>` env pin → 24h disk
//! cache → EXTRA rotation list → shipped baseline. Live rescrape (homepage →
//! up to 800 lazy chunks, 16 workers) runs only via `doctor --refresh` or
//! after a 404 invalidates the cached ID — then one refresh, one retry, then
//! exit 6 (contract-drift).

pub mod cache;
pub mod consts;
pub mod endpoints;
pub mod resolve;
pub mod scrape;

pub use cache::{default_cache_path, CACHE_TTL_SECS};
pub use consts::{
    compact_features, extra_features, fallback_query_id, feature_overrides, seeded_extra_rotation,
    DEFAULT_FEATURES, EXTRA_FALLBACK_IDS, FALLBACK_QUERY_IDS,
};
pub use endpoints::{effective_query_id, load_yaml, EndpointEntry, EndpointsMap};
pub use resolve::{
    env_pin_name, invalidate_op, resolve, rotate_extra, ExtraRotation, QidSource, ResolvedQid,
};
pub use scrape::{MAX_CHUNKS, WORKERS};
