//! Provider traits and implementations for the testing framework.
//!
//! Each provider abstracts an external dependency, allowing tests to inject
//! mock implementations while production code uses real implementations.

mod clock;
mod env;
mod fs;
mod http;
mod rng;
mod secrets;
mod uuid;

pub use clock::{ClockProvider, MockClock, RealClock};
pub use env::{EnvProvider, MockEnv, RealEnv};
pub use fs::{FsProvider, MockFs, RealFs};
pub use http::{HttpProvider, HttpResponse, MockHttp, MockHttpRule, RealHttp};
pub use rng::{MockRng, RealRng, RngProvider};
pub use secrets::{MockSecrets, RealSecrets, SecretsProvider};
pub use uuid::{MockUuid, RealUuid, UuidProvider};
