mod classic;
mod v4;

// Each integration-test crate uses a different subset of the shared facade.
#[allow(unused_imports)]
pub use classic::classic_customer_library;
#[allow(unused_imports)]
pub use v4::{
    v4_customer_application, v4_customer_application_with_entry_points, v4_customer_library,
    v4_customer_specs, v4_incomplete_library,
};
