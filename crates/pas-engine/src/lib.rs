//! PAS engine — v0.2.
//!
//! Adds `libname` support (DUCKDB attach + DIR), library/dataset listing
//! commands, and paginated dataset reads.

use thiserror::Error;

mod datastep;
mod libname;
mod library;
mod macros;
mod procs;
mod query;
mod rewrite;
mod sas_sql;
mod scan;
mod session;
mod split;
mod types;

pub use library::{ColumnInfo, DatasetInfo, DirFormat, Library, LibraryKind};
pub use session::Session;
pub use split::{extract_sql_statements, split_blocks, strip_comments, Block};
pub use types::{Column, DatasetPage, Event, ResultBlock, SourceSpan, Value};

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("duckdb: {0}")]
    DuckDb(#[from] duckdb::Error),
    #[error("libname: {0}")]
    Libname(#[from] libname::LibnameError),
    #[error("data step: {0}")]
    DataStep(#[from] datastep::DataStepError),
    #[error("{0}")]
    Other(String),
}

pub(crate) const MAX_PREVIEW_ROWS: usize = 1000;

pub fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_bare_select() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit("select 1 as a, 'hi' as b;");
        assert!(matches!(evs.last(), Some(Event::Done)));
        assert!(evs.iter().any(|e| matches!(e, Event::Output { .. })));
    }

    #[test]
    fn runs_proc_sql_block() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(
            r#"
            proc sql;
                create table t as select 1 as a union all select 2;
                select count(*) as n from t;
            quit;
            "#,
        );
        let outputs: Vec<_> = evs
            .iter()
            .filter(|e| matches!(e, Event::Output { .. }))
            .collect();
        assert_eq!(outputs.len(), 1, "got events: {:?}", evs);
    }

    #[test]
    fn doubled_quote_inside_string_does_not_break_libref_rewrite() {
        // Regression: the libref rewriter used to terminate strings at
        // the first matching quote, treating SAS-style '' escapes as
        // close-then-open. A literal 'O''Brien' embedded mid-statement
        // followed by a libref reference must still rewrite correctly.
        let s = Session::new_in_memory().unwrap();
        // WORK is always present; reference work.dual via a query that
        // also contains a doubled-quote literal.
        s.submit("create table people as select 'O''Brien' as name, 42 as age;");
        let evs = s.submit("select name from work.people where name = 'O''Brien';");
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        assert!(evs.iter().any(|e| matches!(e, Event::Output { .. })));
    }

    #[test]
    fn surfaces_sql_error() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit("select * from no_such_table;");
        assert!(evs.iter().any(|e| matches!(e, Event::Error { .. })));
    }

    #[test]
    fn create_table_overwrites_on_second_run() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table t as select 1 as x;");
        let evs = s.submit("create table t as select 2 as x;");
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
    }

    #[test]
    fn lists_work_dataset_after_create() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table foo as select 1 as a;");
        let ds = s.list_datasets("work").unwrap();
        assert!(ds.iter().any(|d| d.name == "foo"));
    }

    #[test]
    fn data_null_does_not_create_work_table() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(
            r#"
            data _null_;
                x = 1;
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let ds = s.list_datasets("work").unwrap();
        assert!(
            !ds.iter().any(|d| d.name.eq_ignore_ascii_case("_null_")),
            "{:?}",
            ds
        );
    }

    #[test]
    fn create_into_dir_library_writes_parquet() {
        let dir = tempdir_path();
        std::fs::create_dir_all(&dir).unwrap();
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(&format!(r#"libname out "{}";"#, dir));
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let evs = s.submit("create table out.demo as select 1 as a union all select 2;");
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        assert!(std::path::Path::new(&format!("{}/demo.parquet", dir)).exists());
        // Read back via libref.
        let evs = s.submit("select count(*) as n from out.demo;");
        assert!(
            evs.iter().any(|e| matches!(e, Event::Output { .. })),
            "{:?}",
            evs
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    fn tempdir_path() -> String {
        let p = std::env::temp_dir().join(format!("pas-test-{}", uuid::Uuid::new_v4()));
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn data_step_filters_and_derives() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x union all select 2 union all select 5;");
        let evs = s.submit(
            r#"
            data work.out;
                set src;
                if x > 1;
                y = x * 10;
                msg = cats('hello-', x);
            run;
            "#,
        );
        let errs: Vec<_> = evs
            .iter()
            .filter(|e| matches!(e, Event::Error { .. }))
            .collect();
        assert!(errs.is_empty(), "errors: {:?}", errs);
        let page = s.dataset_page("work", "out", 0, 100, None).unwrap();
        assert_eq!(page.total_rows, 2);
        // Columns: x, y, msg
        let names: Vec<_> = page.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"x"));
        assert!(names.contains(&"y"));
        assert!(names.contains(&"msg"));
    }

    #[test]
    fn data_step_keep_drop() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as a, 2 as b, 3 as c;");
        s.submit("data o; set src; keep a c; run;");
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        let names: Vec<_> = page.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"a") && names.contains(&"c") && !names.contains(&"b"));
    }

    #[test]
    fn data_step_retain_accumulator() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select * from (values (1), (2), (3), (4)) as t(x);");
        let evs = s.submit(
            r#"
            data o;
                set src;
                retain total 0;
                total = total + x;
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 100, None).unwrap();
        assert_eq!(page.total_rows, 4);
        // Last row's `total` should be 1+2+3+4 = 10.
        let total_idx = page.columns.iter().position(|c| c.name == "total").unwrap();
        if let crate::Value::Float(t) = &page.rows[3][total_idx] {
            assert!((t - 10.0).abs() < 1e-9, "expected 10, got {}", t);
        } else {
            panic!("expected float, got {:?}", page.rows[3][total_idx]);
        }
    }

    #[test]
    fn data_step_by_first_last() {
        let s = Session::new_in_memory().unwrap();
        s.submit(
            "create table src as select * from (values \
             ('a',1),('a',2),('b',1),('c',1),('c',2),('c',3)) as t(grp, x);",
        );
        // Keep only the last row per group.
        let evs = s.submit(
            r#"
            data o;
                set src;
                by grp;
                if last.grp;
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 100, None).unwrap();
        assert_eq!(page.total_rows, 3);
    }

    #[test]
    fn data_step_array_and_do_loop() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as a1, 2 as a2, 3 as a3;");
        let evs = s.submit(
            r#"
            data o;
                set src;
                array a{3} a1 a2 a3;
                total = 0;
                do i = 1 to 3;
                    total = total + a{i};
                end;
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 100, None).unwrap();
        let total_idx = page.columns.iter().position(|c| c.name == "total").unwrap();
        if let crate::Value::Float(t) = &page.rows[0][total_idx] {
            assert!((t - 6.0).abs() < 1e-9);
        } else {
            panic!("not float: {:?}", page.rows[0][total_idx]);
        }
    }

    #[test]
    fn data_step_infile_csv() {
        let dir = tempdir_path();
        std::fs::create_dir_all(&dir).unwrap();
        let path = format!("{}/people.csv", dir);
        std::fs::write(&path, "name,age\nalice,30\nbob,25\ncarol,41\n").unwrap();
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(&format!(
            r#"
            data work.people;
                infile '{}' dlm=',' dsd firstobs=2;
                input name $ age;
            run;
            "#,
            path
        ));
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "people", 0, 100, None).unwrap();
        assert_eq!(page.total_rows, 3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn data_step_put_formats() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x;");
        let evs = s.submit(
            r#"
            data o;
                set src;
                d_fmt = put('15FEB2024'd, 'date9.');
                iso   = put('15FEB2024'd, 'yymmdd10.');
                money = put(1234567.89, 'comma14.2');
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        let by = |n: &str| page.columns.iter().position(|c| c.name == n).unwrap();
        let txt = |idx: usize| match &page.rows[0][idx] {
            crate::Value::Text(s) => s.clone(),
            other => panic!("expected text, got {:?}", other),
        };
        assert_eq!(txt(by("d_fmt")), "15FEB2024");
        assert_eq!(txt(by("iso")), "2024-02-15");
        assert_eq!(txt(by("money")).trim(), "1,234,567.89");
    }

    #[test]
    fn data_step_input_function() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select '01JAN2024' as s;");
        let evs = s.submit(
            r#"
            data o;
                set src;
                d = input(s, 'date9.');
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        let d_idx = page.columns.iter().position(|c| c.name == "d").unwrap();
        let expected = (chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
            - chrono::NaiveDate::from_ymd_opt(1960, 1, 1).unwrap())
        .num_days() as f64;
        match &page.rows[0][d_idx] {
            crate::Value::Float(f) => assert!((f - expected).abs() < 1e-9, "{} vs {}", f, expected),
            other => panic!("expected float, got {:?}", other),
        }
    }

    #[test]
    fn data_step_datalines_input() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(
            "data work.people;\n  input name $ age;\n  datalines;\nalice 30\nbob 25\ncarol 41\n;\nrun;\n",
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "people", 0, 100, None).unwrap();
        assert_eq!(page.total_rows, 3);
        let name_i = page.columns.iter().position(|c| c.name == "name").unwrap();
        let age_i = page.columns.iter().position(|c| c.name == "age").unwrap();
        let names: Vec<String> = page
            .rows
            .iter()
            .map(|r| match &r[name_i] {
                crate::Value::Text(s) => s.clone(),
                _ => String::new(),
            })
            .collect();
        let ages: Vec<f64> = page
            .rows
            .iter()
            .map(|r| match &r[age_i] {
                crate::Value::Float(f) => *f,
                _ => f64::NAN,
            })
            .collect();
        assert_eq!(names, vec!["alice", "bob", "carol"]);
        assert_eq!(ages, vec![30.0, 25.0, 41.0]);
    }

    #[test]
    fn data_step_merge_streams_through_cursors() {
        // 200K left + 200K right with overlapping by-keys. Old impl held
        // both fully materialized in Rust HashMaps. New impl snapshots
        // each to a DuckDB temp table and streams 4K rows at a time per
        // cursor.
        let s = Session::new_in_memory().unwrap();
        s.submit("create table lefts  as select x as id, x * 2  as a from range(0, 200000) t(x);");
        s.submit("create table rights as select x as id, x * 10 as b from range(0, 200000) t(x);");
        let evs = s.submit(
            r#"
            data merged;
                merge lefts rights;
                by id;
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "merged", 0, 1, None).unwrap();
        assert_eq!(page.total_rows, 200000);
    }

    #[test]
    fn tier2_regex_functions() {
        // End-to-end exercise of `prxmatch` and `prxchange`. SAS-style
        // `\N` capture references are normalized to Rust's `$N` by the
        // engine's `convert_repl`, so users can copy/paste familiar
        // patterns directly.
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x;");
        let evs = s.submit(
            r#"
            data o;
                set src;
                /* prxmatch returns 1-based position or 0 */
                pos      = prxmatch('/quick/', 'the quick brown fox');
                pos_i    = prxmatch('/QUICK/i', 'the quick brown fox');
                no_match = prxmatch('/zzz/', 'the quick brown fox');

                /* prxchange: times=-1 means replace all */
                replaced = prxchange('s/foo/bar/', -1, 'foo foo baz');
                /* times=1 stops after the first substitution */
                limited  = prxchange('s/foo/bar/', 1, 'foo foo baz');

                /* Capture groups with SAS-style backreferences */
                captured = prxchange('s/(\w+)@(\w+)/\2 at \1/', 1, 'ada@lovelace');

                /* Case-insensitive global substitution */
                ci_repl  = prxchange('s/HELLO/hi/i', -1, 'Hello, HELLO!');
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        let by = |n: &str| page.columns.iter().position(|c| c.name == n).unwrap();
        let num = |n: &str| match &page.rows[0][by(n)] {
            crate::Value::Float(f) => *f,
            crate::Value::Int(i) => *i as f64,
            other => panic!("{}: expected number, got {:?}", n, other),
        };
        let txt = |n: &str| match &page.rows[0][by(n)] {
            crate::Value::Text(s) => s.clone(),
            other => panic!("{}: expected text, got {:?}", n, other),
        };
        // "the quick brown fox" — "quick" starts at position 5.
        assert_eq!(num("pos"), 5.0);
        assert_eq!(num("pos_i"), 5.0);
        assert_eq!(num("no_match"), 0.0);
        assert_eq!(txt("replaced"), "bar bar baz");
        assert_eq!(txt("limited"), "bar foo baz");
        assert_eq!(txt("captured"), "lovelace at ada");
        assert_eq!(txt("ci_repl"), "hi, hi!");
    }

    #[test]
    fn tier1_function_library() {
        // One golden program that exercises every tier-1 addition end
        // to end. Inputs are crafted so the expected outputs are easy to
        // read off without computing dates by hand.
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x;");
        let evs = s.submit(
            r#"
            data o;
                set src;
                /* string */
                word3 = scan('alpha beta gamma delta', 3);
                where_be = find('the quick brown fox', 'brown');
                no_dashes = tranwrd('a-b-c', '-', ':');
                masked = translate('hello', '*', 'l');
                blamt = compbl('one   two    three');
                title = propcase('the quick brown fox');
                rev = reverse('abc');
                rep = repeat('ab', 2);
                /* numeric */
                sgn_pos = sign(7);
                sgn_neg = sign(-3);
                largest_3 = largest(2, 5, 1, 9, 3);
                smallest_2 = smallest(2, 5, 1, 9, 3);
                ifn_val = ifn(x > 0, 100, -1);
                ifc_val = ifc(x > 0, 'pos', 'neg');
                pos = whichn(9, 5, 1, 9, 3);
                cpos = whichc('b', 'a', 'b', 'c');
                /* missing */
                nm = notmissing(x);
                /* date — yrdif from 1JAN2000 to 1JAN2025 ≈ 25.0 */
                years = yrdif('01JAN2000'd, '01JAN2025'd, 'act/365');
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        let by = |n: &str| page.columns.iter().position(|c| c.name == n).unwrap();
        let txt = |n: &str| match &page.rows[0][by(n)] {
            crate::Value::Text(s) => s.clone(),
            other => panic!("{}: expected text, got {:?}", n, other),
        };
        let num = |n: &str| match &page.rows[0][by(n)] {
            crate::Value::Float(f) => *f,
            crate::Value::Int(i) => *i as f64,
            other => panic!("{}: expected number, got {:?}", n, other),
        };
        assert_eq!(txt("word3"), "gamma");
        assert_eq!(num("where_be"), 11.0);
        assert_eq!(txt("no_dashes"), "a:b:c");
        assert_eq!(txt("masked"), "he**o");
        assert_eq!(txt("blamt"), "one two three");
        assert_eq!(txt("title"), "The Quick Brown Fox");
        assert_eq!(txt("rev"), "cba");
        assert_eq!(txt("rep"), "ababab"); // repeat 'ab' with n=2 → 3 copies
        assert_eq!(num("sgn_pos"), 1.0);
        assert_eq!(num("sgn_neg"), -1.0);
        assert_eq!(num("largest_3"), 5.0); // 2nd largest of {5,1,9,3}
        assert_eq!(num("smallest_2"), 3.0); // 2nd smallest of {5,1,9,3}
        assert_eq!(num("ifn_val"), 100.0);
        assert_eq!(txt("ifc_val"), "pos");
        assert_eq!(num("pos"), 3.0);
        assert_eq!(num("cpos"), 2.0);
        assert_eq!(num("nm"), 1.0);
        // 25 years ÷ 365-basis ≈ 9131/365 = 25.0164…
        assert!((num("years") - 25.0).abs() < 0.05, "got {}", num("years"));
    }

    #[test]
    fn runtime_call_error_carries_span() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x;");
        let program = "data o;\n  set src;\n  y = some_function_that_doesnt_exist(x);\nrun;\n";
        let evs = s.submit(program);
        let span = evs.iter().find_map(|e| match e {
            Event::Error { source_span, .. } => source_span.clone(),
            _ => None,
        });
        let span = span.expect("expected runtime error span");
        // Line 3 is the assignment with the bad function.
        assert_eq!(span.start_line, 3);
        assert!(
            span.start_col >= 5,
            "expected the call to be past the indent, got col {}",
            span.start_col
        );
    }

    #[test]
    fn runtime_array_out_of_range_carries_span() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x;");
        let program = "data o;\n  set src;\n  array a{3} a1 a2 a3;\n  y = a{5};\nrun;\n";
        let evs = s.submit(program);
        let span = evs.iter().find_map(|e| match e {
            Event::Error { source_span, .. } => source_span.clone(),
            _ => None,
        });
        let span = span.expect("expected runtime error span");
        // Line 4 is `y = a{5};`.
        assert_eq!(span.start_line, 4);
    }

    #[test]
    fn proc_sql_error_carries_source_span() {
        let s = Session::new_in_memory().unwrap();
        // FRM instead of FROM — DuckDB emits `LINE 1: SELECT * FRM ...^`.
        let evs = s.submit("select * frm sqlite_master;\n");
        let span = evs.iter().find_map(|e| match e {
            Event::Error { source_span, .. } => source_span.clone(),
            _ => None,
        });
        let span = span.expect("expected PROC SQL error span");
        assert_eq!(span.start_line, 1);
        // Column should be somewhere around the offending `frm` token
        // (col 10 in `select * frm ...` — DuckDB's caret tends to land
        // there). Don't pin to an exact number to stay robust across
        // DuckDB versions, but it must be past the first column.
        assert!(span.start_col >= 5, "got col {}", span.start_col);
    }

    #[test]
    fn data_step_parse_error_carries_source_span() {
        let s = Session::new_in_memory().unwrap();
        // Bad syntax inside the body: missing semicolon after the assignment.
        let program = "data out;\n  set src;\n  x = 1\n  y = 2;\nrun;\n";
        let evs = s.submit(program);
        let err_span = evs.iter().find_map(|e| match e {
            Event::Error { source_span, .. } => source_span.clone(),
            _ => None,
        });
        let span = err_span.expect("expected an Event::Error with a source_span");
        // The offending token should land somewhere on or after the buggy
        // line 3 (which is the assignment without a trailing semicolon).
        assert!(
            span.start_line >= 3,
            "span at line {} should be >= 3",
            span.start_line
        );
        assert!(span.end_line >= span.start_line);
        assert!(span.start_col >= 1);
    }

    #[test]
    fn data_step_streams_large_input() {
        // 500K rows × a couple of derived columns. Materializing this as
        // Vec<HashMap<String, RtValue>> would cost ~150-200 MB. With the
        // streaming pipeline it should fit easily in normal test memory
        // and finish in a few seconds.
        let s = Session::new_in_memory().unwrap();
        s.submit("create table big as select * from range(0, 500000) t(x);");
        let evs = s.submit(
            r#"
            data work.big_out;
                set big;
                y = x * 2;
                bucket = mod(x, 100);
                if bucket = 0;
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "big_out", 0, 1, None).unwrap();
        assert_eq!(page.total_rows, 5000);
    }

    #[test]
    fn proc_sort_orders_with_nodupkey() {
        let s = Session::new_in_memory().unwrap();
        s.submit(
            "create table src as select * from (values \
             ('b', 2),('a', 1),('a', 3),('c', 5)) as t(grp, val);",
        );
        let evs = s.submit("proc sort data=src out=sorted nodupkey; by grp; run;");
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "sorted", 0, 10, None).unwrap();
        // nodupkey keeps one row per by-group (grp): a, b, c → 3 rows.
        assert_eq!(page.total_rows, 3);
        let grp_idx = page.columns.iter().position(|c| c.name == "grp").unwrap();
        if let crate::Value::Text(g) = &page.rows[0][grp_idx] {
            assert_eq!(g, "a");
        }
    }

    #[test]
    fn proc_print_emits_output_block() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select * from (values (1),(2),(3)) as t(x);");
        let evs = s.submit("proc print data=src obs=2; var x; run;");
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let outputs: Vec<_> = evs
            .iter()
            .filter(|e| matches!(e, Event::Output { .. }))
            .collect();
        assert_eq!(outputs.len(), 1);
        if let Some(Event::Output { block }) = outputs.first() {
            assert_eq!(block.rows.len(), 2);
            assert_eq!(block.columns[0].name, "x");
        }
    }

    #[test]
    fn proc_transpose_pivots_long_to_wide() {
        let s = Session::new_in_memory().unwrap();
        s.submit(
            "create table sales as select * from (values \
             ('east','q1',10),('east','q2',20),('west','q1',5),('west','q2',8)) as t(region, qtr, amount);",
        );
        let evs =
            s.submit("proc transpose data=sales out=wide; by region; id qtr; var amount; run;");
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "wide", 0, 10, None).unwrap();
        // 2 regions × (region + q1 + q2) = 2 rows, 3 columns
        assert_eq!(page.total_rows, 2);
        let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"region"));
        assert!(names.contains(&"q1"));
        assert!(names.contains(&"q2"));
    }

    #[test]
    fn proc_sql_calculated_keyword() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select * from (values (10), (20), (30)) as t(x);");
        let evs = s.submit(
            "proc sql; create table o as select x, x*2 as doubled, calculated doubled + 1 as plus1 from src; quit;",
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        assert_eq!(page.total_rows, 3);
    }

    #[test]
    fn proc_sql_monotonic_function() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select * from (values ('a'),('b'),('c')) as t(letter);");
        let evs = s
            .submit("proc sql; create table o as select monotonic() as rn, letter from src; quit;");
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        assert_eq!(page.total_rows, 3);
        let rn_idx = page.columns.iter().position(|c| c.name == "rn").unwrap();
        // First row's rn should be 1.
        match &page.rows[0][rn_idx] {
            crate::Value::Int(n) => assert_eq!(*n, 1),
            other => panic!("expected int rn, got {:?}", other),
        }
    }

    #[test]
    fn proc_sql_outer_union_corr_aligns_by_name() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table a as select 1 as id, 'ada' as name;");
        s.submit("create table b as select 'alan' as name, 2 as id;");
        let evs = s.submit(
            "proc sql; create table merged as select * from a outer union corr select * from b; quit;",
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "merged", 0, 10, None).unwrap();
        // Two rows, columns id + name regardless of declaration order.
        assert_eq!(page.total_rows, 2);
        let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"id") && names.contains(&"name"));
    }

    #[test]
    fn macro_let_and_put_drive_program() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(
            r#"
            %let target = 42;
            %put answer is &target;
            create table t as select &target as x;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        // The %put text appears as a NOTE.
        assert!(evs
            .iter()
            .any(|e| matches!(e, Event::Note { text } if text.contains("answer is 42"))));
        // The table was created with x = 42.
        let page = s.dataset_page("work", "t", 0, 10, None).unwrap();
        assert_eq!(page.total_rows, 1);
        if let crate::Value::Int(n) = &page.rows[0][0] {
            assert_eq!(*n, 42);
        }
    }

    #[test]
    fn test_proc_sql_into_clause_assigns_macro_vars() {
        let s = Session::new_in_memory().unwrap();
        let evs = s.submit(
            r#"
            proc sql;
                create table raw_employees as
                    select 4500 as salary union all
                    select 6200 as salary;
            quit;

            proc sql noprint;
                select count(*) into :n_employees trimmed
                from raw_employees;
            quit;

            %put n_employees count is &n_employees;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        assert!(
            evs.iter().any(
                |e| matches!(e, Event::Note { text } if text.contains("n_employees count is 2"))
            ),
            "{:?}",
            evs
        );
    }

    #[test]
    fn macro_vars_persist_across_submissions() {
        let s = Session::new_in_memory().unwrap();
        s.submit("%let name = ada;");
        let evs = s.submit("%put hi &name;");
        assert!(evs
            .iter()
            .any(|e| matches!(e, Event::Note { text } if text.contains("hi ada"))));
    }

    #[test]
    fn test_call_symput_and_symputx() {
        let s = Session::new_in_memory().unwrap();
        s.submit(
            r#"
            data _null_;
                call symput('var1', ' hello ');
                call symputx('var2', '  world  ');
            run;
            "#,
        );
        let evs = s.submit("%put val1=&var1 val2=&var2;");
        assert!(evs.iter().any(
            |e| matches!(e, Event::Note { text } if text.contains("val1= hello  val2=world"))
        ));
    }

    #[test]
    fn test_macro_and_symput_integration() {
        let s = Session::new_in_memory().unwrap();
        let _evs1 = s.submit(
            r#"
            %let my_prefix = user;

            %macro test_macro(val);
                %put --- Executing test_macro ---;
                %let processed_val = %upcase(&val);
                
                data work.&my_prefix._data;
                    length name $15 val_str $15;
                    name = "symput_test";
                    val_str = "&processed_val";
                    output;
                run;
            %mend;

            %test_macro(active);
            "#,
        );

        let _evs2 = s.submit(
            r#"
            data _null_;
                set work.user_data;
                call symput('dynamic_var', '  Value from DATA step  ');
                call symputx('dynamic_var_x', '  Value from DATA step  ');
            run;
            "#,
        );

        let evs = s.submit(
            r#"
            %put SYMPUT:  "&dynamic_var";
            %put SYMPUTX: "&dynamic_var_x";
            "#,
        );

        assert!(evs.iter().any(|e| matches!(e, Event::Note { text } if text.contains("SYMPUT:  \"  Value from DATA step  \""))));
        assert!(evs.iter().any(|e| matches!(e, Event::Note { text } if text.contains("SYMPUTX: \"Value from DATA step\""))));
    }

    #[test]
    fn test_proc_sql_work_library_reference() {
        let s = Session::new_in_memory().unwrap();
        s.submit(
            r#"
            data work.sample_items;
                length name $15 status $15;
                name = "Item_1";
                status = "ACTIVE";
                output;
            run;
            "#,
        );

        let evs = s.submit(
            r#"
            proc sql;
                select * from work.sample_items where status = 'ACTIVE';
            quit;
            "#,
        );

        // Verify that the query returned 1 row successfully instead of raising a Catalog Error.
        assert!(evs.iter().any(
            |e| matches!(e, Event::Note { text } if text.contains("Statement returned 1 row(s)"))
        ));
    }

    #[test]
    fn data_step_select_when() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select * from (values (1),(2),(3),(4)) as t(x);");
        let evs = s.submit(
            r#"
            data o;
                set src;
                select (x);
                    when (1) label = 'one';
                    when (2, 3) label = 'middle';
                    otherwise label = 'big';
                end;
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 100, None).unwrap();
        let label_idx = page.columns.iter().position(|c| c.name == "label").unwrap();
        let labels: Vec<String> = page
            .rows
            .iter()
            .map(|r| match &r[label_idx] {
                crate::Value::Text(s) => s.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(labels, vec!["one", "middle", "middle", "big"]);
    }

    #[test]
    fn data_step_date_funcs_and_literal() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 1 as x;");
        let evs = s.submit(
            r#"
            data o;
                set src;
                d  = '01JAN2024'd;
                yr = year(d);
                mn = month(d);
                dy = day(d);
                next = intnx('month', d, 1);
                gap  = intck('year', d, '01JAN2026'd);
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        let by_name = |n: &str| page.columns.iter().position(|c| c.name == n).unwrap();
        let row = &page.rows[0];
        let n = |i: usize| match &row[i] {
            crate::Value::Float(f) => *f,
            _ => f64::NAN,
        };
        assert_eq!(n(by_name("yr")), 2024.0);
        assert_eq!(n(by_name("mn")), 1.0);
        assert_eq!(n(by_name("dy")), 1.0);
        // 01FEB2024 = days from 1960-01-01
        // intnx('month', 01JAN2024, 1) → 01FEB2024
        let feb1 = (chrono::NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
            - chrono::NaiveDate::from_ymd_opt(1960, 1, 1).unwrap())
        .num_days() as f64;
        assert_eq!(n(by_name("next")), feb1);
        assert_eq!(n(by_name("gap")), 2.0);
    }

    #[test]
    fn data_step_merge_one_to_many() {
        let s = Session::new_in_memory().unwrap();
        s.submit(
            "create table people as select * from (values (1,'a'),(2,'b'),(3,'c')) as t(id, name);",
        );
        s.submit(
            "create table scores as select * from (values (1,10),(1,20),(2,30)) as t(id, score);",
        );
        let evs = s.submit(
            r#"
            data o;
                merge people scores;
                by id;
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 100, None).unwrap();
        // id=1 → 2 rows (name=a broadcast), id=2 → 1 row, id=3 → 1 row (score null) = 4 total
        assert_eq!(page.total_rows, 4);
    }

    #[test]
    fn data_step_reads_and_writes_dir_library() {
        let dir = tempdir_path();
        std::fs::create_dir_all(&dir).unwrap();
        let s = Session::new_in_memory().unwrap();
        s.submit(&format!(r#"libname dados "{}";"#, dir));
        // Seed a parquet file via PROC SQL (CREATE TABLE dados.people).
        let evs = s.submit("create table dados.people as select 'Ada' as name, 1815 as born;");
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        // DATA step that reads + writes via the DIR libref.
        let evs = s.submit(
            r#"
            data dados.people_12;
                set dados.people;
                numero_12 = 12;
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        assert!(std::path::Path::new(&format!("{}/people_12.parquet", dir)).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn data_step_if_then_else() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table src as select 5 as x union all select 0 as x;");
        let evs = s.submit(
            r#"
            data o; set src;
                if x > 0 then sign = 'pos'; else sign = 'zero';
            run;
            "#,
        );
        assert!(
            !evs.iter().any(|e| matches!(e, Event::Error { .. })),
            "{:?}",
            evs
        );
        let page = s.dataset_page("work", "o", 0, 10, None).unwrap();
        assert_eq!(page.total_rows, 2);
    }

    #[test]
    fn dataset_page_arrow_produces_ipc_stream() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table foo as select * from range(0, 50) t(x);");
        let bytes = s.dataset_page_arrow("work", "foo", 0, 10, None).unwrap();
        // Arrow IPC stream starts with a 4-byte continuation token 0xFFFFFFFF.
        assert!(bytes.len() > 16, "page should be non-trivial");
        assert_eq!(&bytes[0..4], &[0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn dataset_page_returns_total() {
        let s = Session::new_in_memory().unwrap();
        s.submit("create table foo as select * from range(0, 50) t(x);");
        let page = s.dataset_page("work", "foo", 0, 10, None).unwrap();
        assert_eq!(page.total_rows, 50);
        assert_eq!(page.rows.len(), 10);
    }

    #[test]
    fn test_user_macro_repro() {
        let s = Session::new_in_memory().unwrap();
        let _evs1 = s.submit(
            r#"
            %macro generate_data(lib, dataset, count=3, filter_val=active);
                %let clean_filter = %upcase(&filter_val);
                data &lib..&dataset;
                    length name $15 status $15 category $15;
                    name = "Item_1";
                    status = "&clean_filter";
                    category = "A";
                    output;
                    name = "Item_2";
                    status = "INACTIVE";
                    category = "B";
                    output;
                    name = "Item_3";
                    status = "&clean_filter";
                    category = "A";
                    output;
                run;
            %mend generate_data;
            %generate_data(work, sample_items, count=5, filter_val=active);
            "#,
        );
        let _evs2 = s.submit(
            r#"
            data _null_;
                set work.sample_items;
                if name = 'Item_3' then do;
                    call symput('saved_item', name);
                    call symputx('trimmed_status', status);
                end;
            run;
            "#,
        );
        let _evs3 = s.submit(
            r#"
            %put NOTE: Saved Item Name via SYMPUT:  "&saved_item";
            %put NOTE: Trimmed Status via SYMPUTX: "&trimmed_status";
            "#,
        );
    }
}
