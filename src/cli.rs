const FLAGS: [&str; 9] = [
    "--insecure",
    "--verbose",
    "--no-progress",
    "--resume",
    "--force-small",
    "--force-multipart",
    "--help",
    "-v",
    "-h",
];

const SHORT: [(&str, &str); 3] = [("-c", "--config"), ("-v", "--verbose"), ("-h", "--help")];

/// 返回 (位置参数, 选项列表)
pub fn parse_argv(argv: Vec<String>) -> (Vec<String>, Vec<(String, String)>) {
    let mut pos: Vec<String> = Vec::new();
    let mut opts: Vec<(String, String)> = Vec::new();
    let mut it = argv.into_iter();

    while let Some(a) = it.next() {
        if a == "--" {
            pos.extend(it.by_ref());
            break;
        }
        if a.starts_with("--") {
            let (k, v) = match a.split_once('=') {
                Some((k, v)) => (k.to_string(), v.to_string()),
                None => {
                    if FLAGS.contains(&a.as_str()) {
                        (a.clone(), String::from("1"))
                    } else {
                        match it.next() {
                            Some(n) if !n.starts_with('-') => (a.clone(), n),
                            _ => (a.clone(), String::new()),
                        }
                    }
                }
            };
            opts.push((k, v));
        } else if a.len() == 2 && a.starts_with('-') {
            let mapped = SHORT
                .iter()
                .find(|(s, _)| *s == a.as_str())
                .map(|(_, l)| l.to_string())
                .unwrap_or(a.clone());
            if FLAGS.contains(&mapped.as_str()) {
                opts.push((mapped, String::from("1")));
            } else {
                let v = it.next().unwrap_or_default();
                opts.push((mapped, v));
            }
        } else {
            pos.push(a);
        }
    }
    (pos, opts)
}

pub fn opt(opts: &[(String, String)], name: &str) -> Option<String> {
    opts.iter()
        .rev()
        .find(|(k, v)| k == name && !v.is_empty())
        .map(|(_, v)| v.clone())
}

pub fn has(opts: &[(String, String)], name: &str) -> bool {
    opts.iter().any(|(k, _)| k == name)
}
