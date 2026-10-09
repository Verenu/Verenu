pub fn identity(value: Option<&str>) -> &str {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
}

#[cfg(test)]
mod tests {
    #[test]
    fn unsigned_workflow_values_use_ad_hoc_signing() {
        for value in [None, Some(""), Some("  \t")] {
            assert_eq!(super::identity(value), "-");
        }
        assert_eq!(
            super::identity(Some(" Developer ID Test ")),
            "Developer ID Test"
        );
    }
}
