pub fn validate_release_config(
    required: bool,
    token: Option<&str>,
    host: Option<&str>,
) -> Result<(), &'static str> {
    if !required {
        return Ok(());
    }

    let token = token.unwrap_or_default();
    if token.trim().is_empty() || token.trim() != token || token.chars().any(char::is_whitespace) {
        return Err("release build requires a non-empty POSTHOG_PROJECT_TOKEN");
    }

    let host = host.unwrap_or_default();
    if !host.starts_with("https://") || host.chars().any(char::is_whitespace) {
        return Err("release build requires POSTHOG_HOST to be an HTTPS URL");
    }
    let Ok(parsed_host) = url::Url::parse(host) else {
        return Err("release build requires POSTHOG_HOST to be an HTTPS URL");
    };
    if parsed_host.scheme() != "https"
        || parsed_host.host_str().is_none()
        || !parsed_host.username().is_empty()
        || parsed_host.password().is_some()
        || parsed_host.path() != "/"
        || parsed_host.query().is_some()
        || parsed_host.fragment().is_some()
    {
        return Err("release build requires POSTHOG_HOST to be an HTTPS URL");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_release_config;

    #[test]
    fn local_builds_can_omit_analytics_configuration() {
        assert_eq!(validate_release_config(false, None, None), Ok(()));
    }

    #[test]
    fn release_builds_fail_when_the_ingestion_token_is_missing() {
        assert_eq!(
            validate_release_config(true, Some("  "), Some("https://us.i.posthog.com")),
            Err("release build requires a non-empty POSTHOG_PROJECT_TOKEN")
        );
    }

    #[test]
    fn release_builds_accept_a_configured_ingestion_token_and_https_host() {
        assert_eq!(
            validate_release_config(
                true,
                Some("test-ingestion-token"),
                Some("https://us.i.posthog.com")
            ),
            Ok(())
        );
        assert_eq!(
            validate_release_config(
                true,
                Some("test-ingestion-token"),
                Some("https://us.i.posthog.com/")
            ),
            Ok(())
        );
    }

    #[test]
    fn release_builds_reject_invalid_or_insecure_hosts() {
        for host in [
            None,
            Some("http://us.i.posthog.com"),
            Some("https:// "),
            Some("https://us.i.posthog.com/path"),
            Some("https://user@us.i.posthog.com"),
            Some("https://us.i.posthog.com:bad"),
            Some("https://us.i.posthog.com:99999"),
        ] {
            assert_eq!(
                validate_release_config(true, Some("test-ingestion-token"), host),
                Err("release build requires POSTHOG_HOST to be an HTTPS URL")
            );
        }
    }
}
