/// Client-side bindings for the `lunaris-titlebar-v1` Wayland protocol.
///
/// Generated from `protocols/lunaris-titlebar-v1.xml` via `wayland-scanner`.

#[allow(non_snake_case, non_upper_case_globals, non_camel_case_types)]
pub mod generated {
    use wayland_client::{self, protocol::*};

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        use wayland_backend;
        wayland_scanner::generate_interfaces!(
            "protocols/lunaris-titlebar-v1.xml"
        );
    }

    use self::__interfaces::*;

    wayland_scanner::generate_client_code!(
        "protocols/lunaris-titlebar-v1.xml"
    );
}

pub use generated::lunaris_titlebar_manager_v1;
pub use generated::lunaris_titlebar_v1;
