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
