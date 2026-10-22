pub fn is_macos_intel() -> bool {
    cfg!(all(target_os = "macos", not(target_arch = "aarch64")))
}

#[cfg(test)]
mod tests {
    use super::is_macos_intel;

    #[test]
    fn is_false_on_every_platform_except_intel_macos() {
        #[cfg(not(all(target_os = "macos", not(target_arch = "aarch64"))))]
        assert!(!is_macos_intel());
    }
}
