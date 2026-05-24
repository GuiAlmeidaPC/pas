//! Built-in DATA step functions (v0.3 subset).

use super::exec::{is_missing, RtValue};

pub fn call(name: &str, args: &[RtValue]) -> Result<RtValue, String> {
    match name {
        // ── numeric ────────────────────────────────────────────────────
        "abs" => num1(args, |x| x.abs()),
        "ceil" => num1(args, |x| x.ceil()),
        "floor" => num1(args, |x| x.floor()),
        "int" => num1(args, |x| x.trunc()),
        "sqrt" => num1(args, |x| x.sqrt()),
        "exp" => num1(args, |x| x.exp()),
        "log" => num1(args, |x| x.ln()),
        "log10" => num1(args, |x| x.log10()),
        "log2" => num1(args, |x| x.log2()),
        "round" => {
            let x = arg_num(args, 0)?;
            let unit = if args.len() >= 2 {
                arg_num(args, 1)?
            } else {
                1.0
            };
            if unit == 0.0 {
                return Ok(RtValue::Num(x));
            }
            Ok(RtValue::Num((x / unit).round() * unit))
        }
        "mod" => {
            let a = arg_num(args, 0)?;
            let b = arg_num(args, 1)?;
            if b == 0.0 {
                return Ok(RtValue::missing());
            }
            Ok(RtValue::Num(a - (a / b).trunc() * b))
        }
        "min" => Ok(reduce_num(args, f64::INFINITY, f64::min)),
        "max" => Ok(reduce_num(args, f64::NEG_INFINITY, f64::max)),
        "sum" => Ok(RtValue::Num(args.iter().filter_map(|v| v.as_num()).sum())),
        "mean" => {
            let nums: Vec<f64> = args.iter().filter_map(|v| v.as_num()).collect();
            if nums.is_empty() {
                Ok(RtValue::missing())
            } else {
                Ok(RtValue::Num(nums.iter().sum::<f64>() / nums.len() as f64))
            }
        }
        "sign" => {
            let x = arg_num(args, 0)?;
            Ok(RtValue::Num(if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            }))
        }
        "largest" | "smallest" => {
            // largest(k, v1, v2, …) — k-th largest of the values (1-based).
            // SAS skips missing values when ranking.
            let k = arg_num(args, 0)? as usize;
            if k == 0 || args.len() < 2 {
                return Ok(RtValue::missing());
            }
            let mut nums: Vec<f64> = args[1..].iter().filter_map(|v| v.as_num()).collect();
            if k > nums.len() {
                return Ok(RtValue::missing());
            }
            // sort_unstable_by handles NaN-free f64s; nums has none after
            // filtering out missing.
            if name == "largest" {
                nums.sort_unstable_by(|a, b| b.partial_cmp(a).unwrap());
            } else {
                nums.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
            }
            Ok(RtValue::Num(nums[k - 1]))
        }
        "ifn" => {
            // ifn(cond, t, f) → numeric ternary
            let cond = args.first().map(|v| v.truthy()).unwrap_or(false);
            Ok(args
                .get(if cond { 1 } else { 2 })
                .cloned()
                .unwrap_or_else(RtValue::missing))
        }

        // ── string ─────────────────────────────────────────────────────
        "upcase" => Ok(RtValue::Str(arg_str(args, 0)?.to_uppercase())),
        "lowcase" => Ok(RtValue::Str(arg_str(args, 0)?.to_lowercase())),
        "length" => {
            // SAS length returns the position of the rightmost non-blank.
            let s = arg_str(args, 0)?;
            let trimmed = s.trim_end_matches(' ');
            Ok(RtValue::Num(trimmed.chars().count() as f64))
        }
        "lengthc" => Ok(RtValue::Num(arg_str(args, 0)?.chars().count() as f64)),
        "lengthn" => {
            let s = arg_str(args, 0)?;
            Ok(RtValue::Num(s.trim_end_matches(' ').chars().count() as f64))
        }
        "trim" => Ok(RtValue::Str(arg_str(args, 0)?.trim_end().to_string())),
        "strip" => Ok(RtValue::Str(arg_str(args, 0)?.trim().to_string())),
        "left" => {
            let s = arg_str(args, 0)?;
            Ok(RtValue::Str(s.trim_start().to_string()))
        }
        "substr" => {
            let s = arg_str(args, 0)?;
            let start = arg_num(args, 1)? as isize;
            let chars: Vec<char> = s.chars().collect();
            if start < 1 {
                return Ok(RtValue::Str(String::new()));
            }
            let start_idx = (start - 1) as usize;
            if start_idx >= chars.len() {
                return Ok(RtValue::Str(String::new()));
            }
            let len = if args.len() >= 3 {
                arg_num(args, 2)? as usize
            } else {
                chars.len() - start_idx
            };
            let end = (start_idx + len).min(chars.len());
            Ok(RtValue::Str(chars[start_idx..end].iter().collect()))
        }
        "cats" => Ok(RtValue::Str(
            args.iter()
                .map(|v| v.as_str().trim().to_string())
                .collect::<Vec<_>>()
                .join(""),
        )),
        "catx" => {
            if args.is_empty() {
                return Ok(RtValue::Str(String::new()));
            }
            let sep = args[0].as_str().to_string();
            let parts: Vec<String> = args[1..]
                .iter()
                .map(|v| v.as_str().trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Ok(RtValue::Str(parts.join(&sep)))
        }
        "compress" => Ok(RtValue::Str(
            arg_str(args, 0)?
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect(),
        )),
        "compbl" => {
            // Collapse runs of internal whitespace down to a single space;
            // leading whitespace stays untouched (matches SAS).
            let s = arg_str(args, 0)?;
            let mut out = String::with_capacity(s.len());
            let mut last_was_space = false;
            for c in s.chars() {
                if c.is_whitespace() {
                    if !last_was_space {
                        out.push(' ');
                    }
                    last_was_space = true;
                } else {
                    out.push(c);
                    last_was_space = false;
                }
            }
            Ok(RtValue::Str(out))
        }
        "propcase" => {
            let s = arg_str(args, 0)?;
            let mut out = String::with_capacity(s.len());
            let mut capitalize = true;
            for c in s.chars() {
                if c.is_alphabetic() {
                    if capitalize {
                        out.extend(c.to_uppercase());
                        capitalize = false;
                    } else {
                        out.extend(c.to_lowercase());
                    }
                } else {
                    capitalize = !c.is_alphanumeric();
                    out.push(c);
                }
            }
            Ok(RtValue::Str(out))
        }
        "reverse" => Ok(RtValue::Str(arg_str(args, 0)?.chars().rev().collect())),
        "repeat" => {
            // SAS repeat(s, n) returns s repeated (n+1) times. So
            // repeat('a', 2) → 'aaa'.
            let s = arg_str(args, 0)?;
            let n = arg_num(args, 1)? as i64;
            let times = if n < 0 { 0 } else { n as usize + 1 };
            Ok(RtValue::Str(s.repeat(times)))
        }
        "scan" => {
            // scan(s, n, [delim]) — 1-based word index. Negative n
            // counts from the right. Default delim is SAS's wordy set of
            // separators; we use whitespace + common punctuation.
            let s = arg_str(args, 0)?;
            let n = arg_num(args, 1)? as i64;
            let delim: Vec<char> = if args.len() >= 3 {
                arg_str(args, 2)?.chars().collect()
            } else {
                " \t\n\r,.;:!?()/&|\"'".chars().collect()
            };
            let is_delim = |c: char| delim.contains(&c);
            let words: Vec<&str> = s
                .split(|c: char| is_delim(c))
                .filter(|w| !w.is_empty())
                .collect();
            if words.is_empty() || n == 0 {
                return Ok(RtValue::Str(String::new()));
            }
            let idx: usize = if n > 0 {
                (n - 1) as usize
            } else {
                let from_end = (-n) as usize;
                if from_end > words.len() {
                    return Ok(RtValue::Str(String::new()));
                }
                words.len() - from_end
            };
            Ok(RtValue::Str(
                words.get(idx).copied().unwrap_or("").to_string(),
            ))
        }
        "find" => {
            // find(haystack, needle, [start], [modifiers])
            // Start is 1-based; modifiers can include 'i' for
            // case-insensitive. We treat anything past pos 2 as the
            // modifier/string mix and parse both forms.
            let hay = arg_str(args, 0)?;
            let needle = arg_str(args, 1)?;
            if needle.is_empty() {
                return Ok(RtValue::Num(0.0));
            }
            let mut start: usize = 1;
            let mut case_insensitive = false;
            for v in args.iter().skip(2) {
                if let Some(n) = v.as_num() {
                    start = (n as i64).max(1) as usize;
                } else {
                    let s = v.as_str();
                    if s.contains(['i', 'I']) {
                        case_insensitive = true;
                    }
                }
            }
            let chars: Vec<char> = hay.chars().collect();
            if start > chars.len() {
                return Ok(RtValue::Num(0.0));
            }
            let needle_lc = if case_insensitive {
                needle.to_lowercase()
            } else {
                needle.clone()
            };
            let hay_from: String = chars[start - 1..].iter().collect();
            let hay_search = if case_insensitive {
                hay_from.to_lowercase()
            } else {
                hay_from
            };
            match hay_search.find(needle_lc.as_str()) {
                Some(byte_off) => {
                    // Translate byte offset back to char position.
                    let chars_before = hay_search[..byte_off].chars().count();
                    Ok(RtValue::Num((start + chars_before) as f64))
                }
                None => Ok(RtValue::Num(0.0)),
            }
        }
        "tranwrd" => {
            let s = arg_str(args, 0)?;
            let from = arg_str(args, 1)?;
            let to = arg_str(args, 2)?;
            if from.is_empty() {
                return Ok(RtValue::Str(s));
            }
            Ok(RtValue::Str(s.replace(&from, &to)))
        }
        "translate" => {
            // translate(s, to, from) — char-by-char substitution. Each
            // char in `from` maps to the char at the same position in `to`.
            // Excess `from` chars are removed; excess `to` chars ignored.
            let s = arg_str(args, 0)?;
            let to_chars: Vec<char> = arg_str(args, 1)?.chars().collect();
            let from_chars: Vec<char> = arg_str(args, 2)?.chars().collect();
            let out: String = s
                .chars()
                .map(|c| match from_chars.iter().position(|&f| f == c) {
                    Some(i) => to_chars.get(i).copied().unwrap_or(c),
                    None => c,
                })
                .collect();
            Ok(RtValue::Str(out))
        }
        "ifc" => {
            // ifc(cond, t, f) → character ternary
            let cond = args.first().map(|v| v.truthy()).unwrap_or(false);
            let pick = args.get(if cond { 1 } else { 2 });
            Ok(RtValue::Str(pick.map(|v| v.as_str()).unwrap_or_default()))
        }
        "whichn" | "whichc" => {
            // which*(target, v1, v2, …) → 1-based position, 0 if not
            // found. whichn compares numerically when possible; whichc
            // compares as strings.
            let target = args.first().cloned().unwrap_or_else(RtValue::missing);
            let comparator: Box<dyn Fn(&RtValue) -> bool> = if name == "whichn" {
                let target_num = target.as_num();
                Box::new(move |v| v.as_num() == target_num)
            } else {
                let target_str = target.as_str();
                Box::new(move |v| v.as_str() == target_str)
            };
            for (i, v) in args.iter().skip(1).enumerate() {
                if comparator(v) {
                    return Ok(RtValue::Num((i + 1) as f64));
                }
            }
            Ok(RtValue::Num(0.0))
        }
        "index" => {
            let s = arg_str(args, 0)?;
            let needle = arg_str(args, 1)?;
            if needle.is_empty() {
                return Ok(RtValue::Num(0.0));
            }
            Ok(RtValue::Num(match s.find(&needle) {
                Some(i) => (s[..i].chars().count() + 1) as f64,
                None => 0.0,
            }))
        }

        // ── missing / coalesce ─────────────────────────────────────────
        "missing" => {
            let v = args.first().cloned().unwrap_or_else(RtValue::missing);
            Ok(RtValue::Num(if is_missing(&v) { 1.0 } else { 0.0 }))
        }
        "coalesce" => {
            for v in args {
                if !is_missing(v) {
                    return Ok(v.clone());
                }
            }
            Ok(RtValue::missing())
        }
        "coalescec" => {
            for v in args {
                let s = v.as_str();
                if !s.trim().is_empty() {
                    return Ok(RtValue::Str(s.to_string()));
                }
            }
            Ok(RtValue::Str(String::new()))
        }
        "nmiss" => {
            let n = args.iter().filter(|v| is_missing(v)).count();
            Ok(RtValue::Num(n as f64))
        }
        "notmissing" => {
            let v = args.first().cloned().unwrap_or_else(RtValue::missing);
            Ok(RtValue::Num(if is_missing(&v) { 0.0 } else { 1.0 }))
        }

        // ── date / time ────────────────────────────────────────────────
        "today" | "date" => Ok(RtValue::Num(today_sas() as f64)),
        "datetime" => Ok(RtValue::Num(now_sas_datetime())),
        "time" => Ok(RtValue::Num(now_sas_time())),
        "year" => Ok(date_part(args, |d| {
            use chrono::Datelike;
            d.year() as f64
        })),
        "month" => Ok(date_part(args, |d| {
            use chrono::Datelike;
            d.month() as f64
        })),
        "day" => Ok(date_part(args, |d| {
            use chrono::Datelike;
            d.day() as f64
        })),
        "weekday" => Ok(date_part(args, |d| {
            use chrono::Datelike;
            // SAS weekday: Sunday=1 … Saturday=7
            (d.weekday().num_days_from_sunday() + 1) as f64
        })),
        "qtr" => Ok(date_part(args, |d| {
            use chrono::Datelike;
            ((d.month() - 1) / 3 + 1) as f64
        })),
        "mdy" => {
            use chrono::NaiveDate;
            let m = arg_num(args, 0)? as u32;
            let d = arg_num(args, 1)? as u32;
            let y = arg_num(args, 2)? as i32;
            match NaiveDate::from_ymd_opt(y, m, d) {
                Some(date) => Ok(RtValue::Num(
                    (date - NaiveDate::from_ymd_opt(1960, 1, 1).unwrap()).num_days() as f64,
                )),
                None => Ok(RtValue::missing()),
            }
        }
        "hms" => {
            let h = arg_num(args, 0)?;
            let m = arg_num(args, 1)?;
            let s = arg_num(args, 2)?;
            Ok(RtValue::Num(h * 3600.0 + m * 60.0 + s))
        }
        "hour" => Ok(RtValue::Num(
            (arg_num(args, 0)?.rem_euclid(86400.0) / 3600.0).floor(),
        )),
        "minute" => Ok(RtValue::Num(
            ((arg_num(args, 0)?.rem_euclid(86400.0)) % 3600.0 / 60.0).floor(),
        )),
        "second" => Ok(RtValue::Num(arg_num(args, 0)?.rem_euclid(60.0))),
        "datepart" => {
            // datetime (seconds) → date (days)
            let dt = arg_num(args, 0)?;
            Ok(RtValue::Num((dt / 86400.0).floor()))
        }
        "timepart" => {
            let dt = arg_num(args, 0)?;
            Ok(RtValue::Num(dt - (dt / 86400.0).floor() * 86400.0))
        }
        "intnx" => {
            let interval = arg_str(args, 0)?.to_ascii_lowercase();
            let start = arg_num(args, 1)?;
            let n = arg_num(args, 2)? as i64;
            Ok(intnx(&interval, start, n))
        }
        "intck" => {
            let interval = arg_str(args, 0)?.to_ascii_lowercase();
            let a = arg_num(args, 1)?;
            let b = arg_num(args, 2)?;
            Ok(intck(&interval, a, b))
        }
        // ── regex (PRX) ────────────────────────────────────────────────
        // SAS PRX functions accept a "perl regex" string in the form
        //   '/<pattern>/<flags>'           (prxmatch / m-form)
        //   's/<pattern>/<replacement>/<flags>'   (prxchange)
        // Backed by the `regex` crate. Lookaround and backreferences
        // aren't supported (regex crate limitation); everything else is.
        "prxmatch" => {
            let pat = arg_str(args, 0)?;
            let src = arg_str(args, 1)?;
            let (re_pat, flags) = parse_prx_match(&pat)?;
            let re = build_regex(&re_pat, &flags)?;
            Ok(RtValue::Num(match re.find(&src) {
                Some(m) => (src[..m.start()].chars().count() + 1) as f64,
                None => 0.0,
            }))
        }
        "prxchange" => {
            // SAS: prxchange(pattern, times, source); times = -1 → all.
            let pat = arg_str(args, 0)?;
            let times = arg_num(args, 1)? as i64;
            let src = arg_str(args, 2)?;
            let (re_pat, repl, flags) = parse_prx_change(&pat)?;
            let re = build_regex(&re_pat, &flags)?;
            let rust_repl = convert_repl(&repl);
            let result = if times < 0 || flags.contains('g') {
                re.replace_all(&src, rust_repl.as_str()).into_owned()
            } else {
                re.replacen(&src, times as usize, rust_repl.as_str())
                    .into_owned()
            };
            Ok(RtValue::Str(result))
        }
        "yrdif" => {
            // yrdif(d1, d2, basis) — fractional years between two SAS
            // dates. Supported bases: 'act/act' (default — exact day
            // count divided by the calendar-year length spanning the
            // interval, approximated as 365.25), '30/360', 'act/360',
            // 'act/365'.
            let d1 = arg_num(args, 0)?;
            let d2 = arg_num(args, 1)?;
            let basis = args
                .get(2)
                .map(|v| v.as_str().trim().to_ascii_lowercase())
                .unwrap_or_else(|| "act/act".into());
            Ok(yrdif(d1, d2, &basis))
        }

        // ── formatted I/O ──────────────────────────────────────────────
        // NOTE: PAS requires the format spec as a quoted string (e.g.
        // `put(x, 'date9.')`). Standard SAS allows the bare `date9.` token.
        "put" => {
            let v = args.first().cloned().unwrap_or_else(RtValue::missing);
            let spec = arg_str(args, 1)?;
            put_value(&v, &spec).map(RtValue::Str)
        }
        "input" => {
            let s = arg_str(args, 0)?;
            let spec = arg_str(args, 1)?;
            input_value(&s, &spec)
        }

        other => Err(format!(
            "function '{}' is not implemented in PAS v0.5",
            other
        )),
    }
}

// ── format / informat helpers ──────────────────────────────────────────────

struct FormatSpec {
    is_char: bool,
    name: String,
    width: Option<u32>,
    decimals: Option<u32>,
}

fn parse_format_spec(spec: &str) -> Result<FormatSpec, String> {
    let s = spec.trim().trim_end_matches('.');
    let is_char = s.starts_with('$');
    let s = s.trim_start_matches('$');
    // Name is the leading run of letters; the rest is `<width>[.dec]` or empty.
    let name_end = s
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(s.len());
    let name = s[..name_end].to_ascii_lowercase();
    let suffix = &s[name_end..];
    let (width, decimals) = if suffix.is_empty() {
        (None, None)
    } else if let Some(dot) = suffix.find('.') {
        let w = if dot == 0 {
            None
        } else {
            suffix[..dot].parse().ok()
        };
        let d = suffix[dot + 1..].parse().ok();
        (w, d)
    } else {
        (suffix.parse().ok(), None)
    };
    Ok(FormatSpec {
        is_char,
        name,
        width,
        decimals,
    })
}

fn put_value(v: &RtValue, spec: &str) -> Result<String, String> {
    let fmt = parse_format_spec(spec)?;
    if fmt.is_char {
        let s = v.as_str();
        return Ok(match fmt.name.as_str() {
            "char" | "" => {
                let w = fmt.width.unwrap_or(s.chars().count() as u32) as usize;
                let truncated: String = s.chars().take(w).collect();
                format!("{:<width$}", truncated, width = w)
            }
            "upcase" => s.to_uppercase(),
            "lowcase" => s.to_lowercase(),
            other => return Err(format!("unsupported char format: {}", other)),
        });
    }
    let n = v.as_num();
    Ok(match fmt.name.as_str() {
        "" | "best" => match n {
            Some(n) => format_number_plain(n, fmt.width, fmt.decimals),
            None => format_missing(fmt.width),
        },
        "comma" => match n {
            Some(n) => format_number_commas(n, fmt.width, fmt.decimals),
            None => format_missing(fmt.width),
        },
        "date" => match n.and_then(sas_date_to_naive) {
            Some(d) => format_sas_date(d),
            None => ".".to_string(),
        },
        "mmddyy" => match n.and_then(sas_date_to_naive) {
            Some(d) => {
                use chrono::Datelike;
                let w = fmt.width.unwrap_or(10);
                if w == 8 {
                    format!("{:02}/{:02}/{:02}", d.month(), d.day(), d.year() % 100)
                } else {
                    format!("{:02}/{:02}/{:04}", d.month(), d.day(), d.year())
                }
            }
            None => ".".to_string(),
        },
        "ddmmyy" => match n.and_then(sas_date_to_naive) {
            Some(d) => {
                use chrono::Datelike;
                let w = fmt.width.unwrap_or(10);
                if w == 8 {
                    format!("{:02}/{:02}/{:02}", d.day(), d.month(), d.year() % 100)
                } else {
                    format!("{:02}/{:02}/{:04}", d.day(), d.month(), d.year())
                }
            }
            None => ".".to_string(),
        },
        "yymmdd" => match n.and_then(sas_date_to_naive) {
            Some(d) => {
                use chrono::Datelike;
                format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
            }
            None => ".".to_string(),
        },
        "time" => match n {
            Some(secs) => format_sas_time(secs),
            None => ".".to_string(),
        },
        "datetime" => match n {
            Some(dt) => format_sas_datetime(dt),
            None => ".".to_string(),
        },
        other => return Err(format!("unsupported numeric format: {}", other)),
    })
}

fn input_value(s: &str, spec: &str) -> Result<RtValue, String> {
    let fmt = parse_format_spec(spec)?;
    if fmt.is_char {
        return Ok(RtValue::Str(s.to_string()));
    }
    let s = s.trim();
    Ok(match fmt.name.as_str() {
        "" | "best" => s
            .parse::<f64>()
            .map(RtValue::Num)
            .unwrap_or_else(|_| RtValue::missing()),
        "comma" => s
            .replace(',', "")
            .trim()
            .parse::<f64>()
            .map(RtValue::Num)
            .unwrap_or_else(|_| RtValue::missing()),
        "date" => super::lex::parse_sas_date(s)
            .map(RtValue::Num)
            .unwrap_or_else(|_| RtValue::missing()),
        "time" => super::lex::parse_sas_time(s)
            .map(RtValue::Num)
            .unwrap_or_else(|_| RtValue::missing()),
        "datetime" => super::lex::parse_sas_datetime(s)
            .map(RtValue::Num)
            .unwrap_or_else(|_| RtValue::missing()),
        "mmddyy" | "ddmmyy" | "yymmdd" => parse_slashed_date(s, &fmt.name)
            .map(RtValue::Num)
            .unwrap_or_else(|_| RtValue::missing()),
        other => return Err(format!("unsupported informat: {}", other)),
    })
}

fn parse_slashed_date(s: &str, order: &str) -> Result<f64, String> {
    use chrono::NaiveDate;
    let parts: Vec<&str> = s.split(['/', '-', '.']).collect();
    if parts.len() != 3 {
        return Err("expected 3 parts".into());
    }
    let (y, m, d) = match order {
        "mmddyy" => (parts[2], parts[0], parts[1]),
        "ddmmyy" => (parts[2], parts[1], parts[0]),
        "yymmdd" => (parts[0], parts[1], parts[2]),
        _ => unreachable!(),
    };
    let mut y: i32 = y.parse().map_err(|_| "bad year")?;
    if y < 100 {
        y += if y < 50 { 2000 } else { 1900 };
    }
    let m: u32 = m.parse().map_err(|_| "bad month")?;
    let d: u32 = d.parse().map_err(|_| "bad day")?;
    let date = NaiveDate::from_ymd_opt(y, m, d).ok_or("invalid date")?;
    let base = NaiveDate::from_ymd_opt(1960, 1, 1).unwrap();
    Ok((date - base).num_days() as f64)
}

fn format_number_plain(n: f64, width: Option<u32>, decimals: Option<u32>) -> String {
    let s = match decimals {
        Some(d) => format!("{:.*}", d as usize, n),
        None => {
            if n.fract() == 0.0 && n.abs() < 1e16 {
                format!("{}", n as i64)
            } else {
                format!("{}", n)
            }
        }
    };
    match width {
        Some(w) => format!("{:>width$}", s, width = w as usize),
        None => s,
    }
}

fn format_number_commas(n: f64, width: Option<u32>, decimals: Option<u32>) -> String {
    let abs = n.abs();
    let int_part = abs.trunc() as i64;
    let int_str = {
        let s = int_part.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out.chars().rev().collect::<String>()
    };
    let body = match decimals {
        Some(d) if d > 0 => {
            let frac = (abs.fract() * 10f64.powi(d as i32)).round() as i64;
            format!("{}.{:0width$}", int_str, frac, width = d as usize)
        }
        _ => int_str,
    };
    let signed = if n < 0.0 { format!("-{}", body) } else { body };
    match width {
        Some(w) => format!("{:>width$}", signed, width = w as usize),
        None => signed,
    }
}

fn format_missing(width: Option<u32>) -> String {
    match width {
        Some(w) => format!("{:>width$}", ".", width = w as usize),
        None => ".".to_string(),
    }
}

fn format_sas_date(d: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    let m = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ][d.month() as usize - 1];
    format!("{:02}{}{:04}", d.day(), m, d.year())
}

fn format_sas_time(secs: f64) -> String {
    let total = secs.max(0.0) as i64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

fn format_sas_datetime(dt: f64) -> String {
    let days = (dt / 86400.0).floor();
    let secs = dt - days * 86400.0;
    match sas_date_to_naive(days) {
        Some(d) => format!("{}:{}", format_sas_date(d), format_sas_time(secs)),
        None => ".".to_string(),
    }
}

fn today_sas() -> i64 {
    use chrono::Local;
    let today = Local::now().date_naive();
    let base = chrono::NaiveDate::from_ymd_opt(1960, 1, 1).unwrap();
    (today - base).num_days()
}

fn now_sas_datetime() -> f64 {
    use chrono::Local;
    let now = Local::now().naive_local();
    let base = chrono::NaiveDate::from_ymd_opt(1960, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    (now - base).num_milliseconds() as f64 / 1000.0
}

fn now_sas_time() -> f64 {
    use chrono::{Local, Timelike};
    let now = Local::now().time();
    now.num_seconds_from_midnight() as f64 + now.nanosecond() as f64 / 1e9
}

fn sas_date_to_naive(d: f64) -> Option<chrono::NaiveDate> {
    use chrono::NaiveDate;
    let base = NaiveDate::from_ymd_opt(1960, 1, 1).unwrap();
    base.checked_add_signed(chrono::Duration::days(d as i64))
}

fn date_part(args: &[RtValue], f: impl Fn(chrono::NaiveDate) -> f64) -> RtValue {
    match args
        .first()
        .and_then(|v| v.as_num())
        .and_then(sas_date_to_naive)
    {
        Some(d) => RtValue::Num(f(d)),
        None => RtValue::missing(),
    }
}

fn intnx(interval: &str, start: f64, n: i64) -> RtValue {
    use chrono::{Datelike, Duration, NaiveDate};
    let Some(d) = sas_date_to_naive(start) else {
        return RtValue::missing();
    };
    let new_date = match interval {
        "day" => d.checked_add_signed(Duration::days(n)),
        "week" => d.checked_add_signed(Duration::weeks(n)),
        "month" => {
            let total_months = d.year() as i64 * 12 + d.month() as i64 - 1 + n;
            let y = total_months.div_euclid(12) as i32;
            let m = (total_months.rem_euclid(12) + 1) as u32;
            NaiveDate::from_ymd_opt(y, m, d.day().min(28))
        }
        "year" => NaiveDate::from_ymd_opt(d.year() + n as i32, d.month(), d.day().min(28)),
        _ => return RtValue::missing(),
    };
    match new_date {
        Some(nd) => {
            let base = NaiveDate::from_ymd_opt(1960, 1, 1).unwrap();
            RtValue::Num((nd - base).num_days() as f64)
        }
        None => RtValue::missing(),
    }
}

fn intck(interval: &str, a: f64, b: f64) -> RtValue {
    use chrono::Datelike;
    let (Some(da), Some(db)) = (sas_date_to_naive(a), sas_date_to_naive(b)) else {
        return RtValue::missing();
    };
    let n = match interval {
        "day" => (db - da).num_days(),
        "week" => (db - da).num_days() / 7,
        "month" => {
            (db.year() as i64 * 12 + db.month() as i64)
                - (da.year() as i64 * 12 + da.month() as i64)
        }
        "year" => (db.year() as i64) - (da.year() as i64),
        _ => return RtValue::missing(),
    };
    RtValue::Num(n as f64)
}

fn yrdif(d1: f64, d2: f64, basis: &str) -> RtValue {
    use chrono::Datelike;
    let (Some(da), Some(db)) = (sas_date_to_naive(d1), sas_date_to_naive(d2)) else {
        return RtValue::missing();
    };
    let days = (db - da).num_days() as f64;
    let years = match basis {
        "act/360" => days / 360.0,
        "act/365" => days / 365.0,
        "30/360" => {
            // Bond-day convention.
            let d1d = da.day() as f64;
            let d2d = db.day() as f64;
            let d1m = da.month() as f64;
            let d2m = db.month() as f64;
            let d1y = da.year() as f64;
            let d2y = db.year() as f64;
            let d1d = d1d.min(30.0);
            let d2d = if d1d == 30.0 { d2d.min(30.0) } else { d2d };
            (360.0 * (d2y - d1y) + 30.0 * (d2m - d1m) + (d2d - d1d)) / 360.0
        }
        // Default 'act/act' — divide by 365.25 (calendar-year approximation,
        // matches SAS within rounding for typical age calculations).
        _ => days / 365.25,
    };
    RtValue::Num(years)
}

fn num1(args: &[RtValue], f: fn(f64) -> f64) -> Result<RtValue, String> {
    let x = arg_num(args, 0)?;
    Ok(RtValue::Num(f(x)))
}

fn reduce_num(args: &[RtValue], init: f64, f: fn(f64, f64) -> f64) -> RtValue {
    let nums: Vec<f64> = args.iter().filter_map(|v| v.as_num()).collect();
    if nums.is_empty() {
        RtValue::missing()
    } else {
        RtValue::Num(nums.into_iter().fold(init, f))
    }
}

fn arg_num(args: &[RtValue], i: usize) -> Result<f64, String> {
    args.get(i)
        .and_then(|v| v.as_num())
        .ok_or_else(|| format!("expected numeric argument at position {}", i + 1))
}

fn arg_str(args: &[RtValue], i: usize) -> Result<String, String> {
    args.get(i)
        .map(|v| v.as_str().to_string())
        .ok_or_else(|| format!("expected string argument at position {}", i + 1))
}

fn parse_prx_match(pat: &str) -> Result<(String, String), String> {
    if pat.is_empty() {
        return Err("Empty PRX pattern".into());
    }
    let d = pat.chars().next().unwrap();
    let parts: Vec<&str> = pat.split(d).collect();
    if parts.len() < 3 {
        return Err(format!("Invalid PRX pattern: {}", pat));
    }
    let re_pat = parts[1].to_string();
    let flags = parts[2].to_string();
    Ok((re_pat, flags))
}

fn parse_prx_change(pat: &str) -> Result<(String, String, String), String> {
    if !pat.starts_with('s') {
        return Err("PRX change pattern must start with 's'".into());
    }
    let d = pat.chars().nth(1).unwrap_or('/');
    let parts: Vec<&str> = pat[1..].split(d).collect();
    if parts.len() < 4 {
        return Err(format!("Invalid PRX change pattern: {}", pat));
    }
    let re_pat = parts[1].to_string();
    let repl = parts[2].to_string();
    let flags = parts[3].to_string();
    Ok((re_pat, repl, flags))
}

fn build_regex(re_pat: &str, flags: &str) -> Result<regex::Regex, String> {
    let mut builder = regex::RegexBuilder::new(re_pat);
    for c in flags.chars() {
        match c {
            'i' => {
                builder.case_insensitive(true);
            }
            'm' => {
                builder.multi_line(true);
            }
            's' => {
                builder.dot_matches_new_line(true);
            }
            'x' => {
                builder.ignore_whitespace(true);
            }
            'o' | 'g' => {}
            _ => return Err(format!("Unsupported PRX flag: {}", c)),
        }
    }
    builder.build().map_err(|e| e.to_string())
}

/// Convert a SAS / Perl-style replacement string into the syntax Rust's
/// `regex::Regex::replace*` expects:
///
///   `\1` … `\9` → `$1` … `$9`        (capture group references)
///   `\\`        → `\`                  (literal backslash)
///   `\X` (other)→ `X`                  (drop the backslash)
///   `$`         → `$$`                 (escape — `$` is metacharacter in
///                                       Rust's replacement language)
fn convert_repl(repl: &str) -> String {
    let mut out = String::with_capacity(repl.len());
    let mut chars = repl.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek().copied() {
                Some(d) if d.is_ascii_digit() => {
                    out.push('$');
                    out.push(d);
                    chars.next();
                }
                Some('\\') => {
                    out.push('\\');
                    chars.next();
                }
                Some(other) => {
                    out.push(other);
                    chars.next();
                }
                None => out.push('\\'),
            },
            '$' => out.push_str("$$"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod convert_repl_tests {
    use super::convert_repl;

    #[test]
    fn slash_digits_become_dollar_digits() {
        assert_eq!(convert_repl(r"\2 at \1"), "$2 at $1");
    }

    #[test]
    fn literal_dollar_gets_escaped() {
        assert_eq!(convert_repl("price $5"), "price $$5");
    }

    #[test]
    fn double_backslash_is_literal() {
        assert_eq!(convert_repl(r"a\\b"), r"a\b");
    }
}
