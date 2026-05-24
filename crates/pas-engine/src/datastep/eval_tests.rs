use super::exec::{is_missing, test_eval_expression, RtValue};
use std::collections::HashMap;

fn eval_expr(expr: &str, vars: &[(&str, RtValue)]) -> RtValue {
    let mut map = HashMap::new();
    for (name, val) in vars {
        map.insert(name.to_string(), val.clone());
    }
    test_eval_expression(expr, &map).expect("failed to evaluate expression")
}

#[test]
fn test_string_to_number_coercions() {
    // String addition (implicit conversion)
    let res = eval_expr(
        "x + y",
        &[
            ("x", RtValue::Str(" 12.5 ".to_string())),
            ("y", RtValue::Num(2.0)),
        ],
    );
    if let RtValue::Num(n) = res {
        assert!((n - 14.5).abs() < 1e-9);
    } else {
        panic!("expected Num, got {:?}", res);
    }

    // Number addition coerced from strings
    let res2 = eval_expr(
        "x * y",
        &[
            ("x", RtValue::Str("3".to_string())),
            ("y", RtValue::Str("4".to_string())),
        ],
    );
    if let RtValue::Num(n) = res2 {
        assert!((n - 12.0).abs() < 1e-9);
    } else {
        panic!("expected Num, got {:?}", res2);
    }
}

#[test]
fn test_number_to_string_coercion() {
    // Implicit conversion during concatenation
    let res = eval_expr(
        "x || y",
        &[
            ("x", RtValue::Num(100.0)),
            ("y", RtValue::Str("pcs".to_string())),
        ],
    );
    assert_eq!(res.as_str(), "100pcs");
}

#[test]
fn test_multi_variable_arithmetic_precedence() {
    let vars = &[
        ("x", RtValue::Num(2.0)),
        ("y", RtValue::Num(3.0)),
        ("z", RtValue::Num(4.0)),
    ];

    // Precedence: x + y * z => 2 + 12 = 14
    let res1 = eval_expr("x + y * z", vars);
    if let RtValue::Num(n) = res1 {
        assert!((n - 14.0).abs() < 1e-9);
    } else {
        panic!("expected Num, got {:?}", res1);
    }

    // Precedence: (x + y) * z => 5 * 4 = 20
    let res2 = eval_expr("(x + y) * z", vars);
    if let RtValue::Num(n) = res2 {
        assert!((n - 20.0).abs() < 1e-9);
    } else {
        panic!("expected Num, got {:?}", res2);
    }
}

#[test]
fn test_logical_evaluations() {
    let vars = &[
        ("a", RtValue::Num(1.0)),
        ("b", RtValue::Num(0.0)),
        ("s1", RtValue::Str("abc".to_string())),
        ("s2", RtValue::Str("def".to_string())),
    ];

    // basic boolean checks
    assert_eq!(eval_expr("a and b", vars).truthy(), false);
    assert_eq!(eval_expr("a or b", vars).truthy(), true);
    assert_eq!(eval_expr("not b", vars).truthy(), true);

    // comparison operator checks
    assert_eq!(eval_expr("s1 = 'abc'", vars).truthy(), true);
    assert_eq!(eval_expr("s1 != s2", vars).truthy(), true);
    assert_eq!(eval_expr("a > b", vars).truthy(), true);
    assert_eq!(eval_expr("a < b", vars).truthy(), false);
}

#[test]
fn test_missing_value_propagation() {
    let vars = &[("x", RtValue::Num(5.0)), ("y", RtValue::missing())];

    // Math on missing values propagates missing (NaN)
    let res = eval_expr("x + y", vars);
    assert!(is_missing(&res));

    // logical behavior with missing values (missing is treated as less than any valid value)
    let res_cmp = eval_expr("x > y", vars);
    assert_eq!(res_cmp.truthy(), true);
    let res_cmp2 = eval_expr("x < y", vars);
    assert_eq!(res_cmp2.truthy(), false);
}
