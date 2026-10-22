pub fn insertion_cases() -> Vec<(String, String, String, String)> {
    let mut cases: Vec<(String, String, String, String)> = Vec::new();
    let mut add = |text: &str, left: &str, right: &str, expected: &str| {
        cases.push((text.into(), left.into(), right.into(), expected.into()));
    };
    for end in [
        ".", "!", "?", "?!", "!!!", "…", "...", "。", "！", "？", "؟", "۔", "‼", "⁉",
    ] {
        for close in [
            "", "\"", "'", "”", "’", ")", "]", "»", "\")", "**", "_", "~",
        ] {
            for space in ["", " ", "\t", "\u{a0}"] {
                let left = format!("Done{end}{close}{space}");
                let expected = if space.is_empty() { " Next" } else { "Next" };
                add("next", &left, "", expected);
            }
        }
    }
    for left in [
        "",
        " ",
        "\t",
        "unfinished\n  ",
        "unfinished\r\n\t",
        "Items:\n- ",
        "Items:\n2. ",
    ] {
        add("next", left, "", "Next");
    }
    for left in [
        "Ask Dr.",
        "Use e.g.",
        "Ask Prof.",
        "Use i.e.",
        "Ask J.",
        "The U.S.",
        "She has a Ph.D.",
        "Earned a B.Sc.",
        "Earned an M.Sc.",
        "Earned a D.Sc.",
        "Earned a B.Com.",
    ] {
        add("Next", left, "", " next");
    }
    for left in [
        "Visit example.com.",
        "Release v1.2.",
        "Pay 3.14.",
        "Done...",
    ] {
        add("next", left, "", " Next");
    }
    for prefix in [". ", "! ", "? ", "?! ", "… ", "\n", "\n  "] {
        add(
            &format!("{prefix}next"),
            "unfinished",
            "",
            &format!("{prefix}Next"),
        );
    }
    for (text, left, right, expected) in [
        ("Next", "a \"quote\"", "", " next"),
        ("and left", "She said, \"done,\"", "", " and left"),
        ("and left", "She said, 'done,'", "", " and left"),
        ("and left", "It's 'done,'", "", " and left"),
        ("e.g. this works", "Done.", "", " E.g. this works"),
        ("i.e. this works", "Done.", "", " I.e. this works"),
        ("hello", "They said, \"", "", "Hello"),
        ("value", "call(", ")", "value"),
        ("value", "path/", "file", "value"),
        ("domain", "name@", ".com", "domain"),
        ("'s ready", "it", "", "'s ready"),
        ("n't ready", "is", "", "n't ready"),
        (", however", "word", "", ", however"),
        ("…", "word", "", "…"),
        ("; next", "word", "", "; next"),
        ("new", "Alpha", "beta", " new "),
        ("done.", "", "Next", "Done. "),
        ("word", "", ",", "Word"),
        ("word", "", ")", "Word"),
        ("word", "", "(aside)", "Word "),
        ("word", "", "“quote”", "Word "),
        ("word", "", "'s", "Word"),
        ("hello\"", "", "next", "Hello\" "),
        ("NASA agrees", "They", "", " NASA agrees"),
        ("iPhone works", "Done.", "", " iPhone works"),
        ("I agree", "They", "", " I agree"),
        ("https://example.com", "Done.", "", " https://example.com"),
        ("name@example.com", "", "", "name@example.com"),
        ("snake_case", "", "", "snake_case"),
        ("$skill-name", "", "", "$skill-name"),
        ("@alice", "", "", "@alice"),
        ("@alice", "Hello", "", " @alice"),
        ("@example.com", "name", "", "@example.com"),
        ("@example.com", "Hello", "", " @example.com"),
        ("@alice.dev", "Hello", "", " @alice.dev"),
        ("@example..com", "name", "", " @example..com"),
        ("#topic", "Discuss", "", " #topic"),
        ("#section", "/docs/page+", "", "#section"),
        ("#section", "docs/page", "", "#section"),
        ("#section", "(/docs/page+", "", "#section"),
        // A hyphen fragment is ambiguous with a single-dash option. Keep the
        // prose-boundary spacing rather than guessing from a word list.
        ("-known", "well", "", " -known"),
        ("www.example.com", "Done.", "", " www.example.com"),
        ("example.com", "Done.", "", " example.com"),
        ("--help", "Done.", "", " --help"),
        ("-help", "please visit", "", " -help"),
        (".env", "please use ", "", ".env"),
        (".github", "please use ", "", ".github"),
        (".com", "example", "", ".com"),
        (".de", "münich", "", ".de"),
        ("and/or", "Done.", "", " And/or"),
        ("either/or", "Done.", "", " Either/or"),
        ("input/output", "Done.", "", " input/output"),
        ("münich.de", "Done.", "", " münich.de"),
        ("example.com؟", "Done.", "", " example.com؟"),
        ("(Example.com)", "please visit", "", " (Example.com)"),
        ("[Example.com]", "please visit", "", " [Example.com]"),
        ("\"Example.com\"", "please visit", "", " \"Example.com\""),
        ("bar", "\"foo ", "\",\"next\"", "bar"),
        ("bar", "'foo ", "','next'", "bar"),
        ("bar", "\"foo ", "\"and \"next\"", "bar"),
        ("word", "", "\"quoted text\"", "Word "),
        ("word", "", "'quoted text'", "Word "),
        ("word", "", "\"", "Word"),
        ("word", "", "'", "Word"),
        ("New York works", "They", "", " New York works"),
        ("", "word", "word", ""),
        (" ", "word", "word", " "),
    ] {
        add(text, left, right, expected);
    }
    for token in [
        "Example.com",
        "www.example.com",
        "münich.de",
        ".env",
        ".github",
        "--help",
        "-help",
        "@alice",
        "#topic",
        "$skill-name",
        "snake_case",
        "name@example.com",
    ] {
        for (open, close) in [
            ("", ""),
            ("(", ")"),
            ("[", "]"),
            ("\"", "\""),
            ("'", "'"),
            ("“", "”"),
        ] {
            for ending in [
                "", ".", "!", "?", "…", "。", "！", "？", "؟", "۔", "‼", "⁉", ",", ";", ":",
            ] {
                for left in ["Done.", "please use"] {
                    let text = format!("{open}{token}{close}{ending}");
                    let gap = if text.starts_with('.') { "" } else { " " };
                    add(&text, left, "following", &format!("{gap}{text} "));
                }
            }
        }
    }
    let originals = cases.clone();
    for (text, left, right, expected) in originals {
        if right.is_empty()
            && !text.is_empty()
            && !text.ends_with(char::is_whitespace)
            && !text.ends_with(['_', '/', '-', '@'])
        {
            cases.push((
                text.clone(),
                left.clone(),
                "following".into(),
                format!("{expected} "),
            ));
            cases.push((
                text.clone(),
                left.clone(),
                " following".into(),
                expected.clone(),
            ));
            cases.push((text, left, ")".into(), expected));
        }
    }
    cases
}
