#[cfg(any(test, target_os = "linux"))]
mod broker_admission_error;
#[cfg(any(test, target_os = "linux"))]
mod broker_frame;
#[cfg(test)]
mod broker_frame_tests;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod held_installation_image;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod held_installation_image_tests;
mod installation_config;
#[cfg(test)]
mod installation_config_tests;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod root_installation_path;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use held_installation_image::HeldInstallationImage;
pub use installation_config::InstallationConfig;

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod linux_broker;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod linux_broker_client;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod linux_broker_launch_result;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod original_client_output;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use linux_broker::LinuxBroker;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use linux_broker_client::run_client;
