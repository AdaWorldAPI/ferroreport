//! Groovy expressions (language="groovy") with the text JasperReports 6.20.6 printed for each,
//! running Groovy 4.0.8 on Java 17: lists, maps, ranges, closures, GStrings, Groovy's
//! arithmetic and operators.

use ferroreport::expr::{eval, parse_expr, EvalEnv, Scope};
use ferroreport::format::{format_value, locale::EN_US};
use ferroreport::value::Value;

/// The fields of the row the field expressions were printed with.
struct Row;

impl Scope for Row {
    fn field(&self, n: &str) -> ferroreport::Result<Value> {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 3, 5).unwrap().and_hms_opt(10, 30, 0).unwrap();
        Ok(match n {
            "name" => Value::str("ana pérez"),
            "amount" => Value::Decimal("1234.50".parse().unwrap()),
            "qty" => Value::Int(3),
            "price" => Value::Double(2.5),
            "when" => Value::Date(date),
            "nothing" => Value::Null,
            _ => return Err(ferroreport::Error::Eval(format!("no field {n}"))),
        })
    }
    fn param(&self, n: &str) -> ferroreport::Result<Value> {
        Err(ferroreport::Error::Eval(format!("no parameter {n}")))
    }
    fn variable(&self, n: &str) -> ferroreport::Result<Value> {
        Err(ferroreport::Error::Eval(format!("no variable {n}")))
    }
}

const JASPER: &[(&str, &str)] = &[
    ("[5, 6, 7, 8, 9].findAll{it % 2 == 1}", "[5, 7, 9]"),
    ("[1, 2, 3].collect{it * 2}", "[2, 4, 6]"),
    ("[1, 2, 3].sum()", "6"),
    ("[3, 1, 2].sort()", "[1, 2, 3]"),
    ("\"a,b\".split(\",\").size()", "2"),
    ("\"x${1 + 2}y\"", "x3y"),
    ("[1, 2, 3].collect{ x -> x * 10 }", "[10, 20, 30]"),
    ("[1.5, 2.5].sum()", "4.0"),
    ("[1, 2, 3].sum{it * 2}", "12"),
    ("[\"a\", \"b\"].sum()", "ab"),
    ("[3, 1, 2].sort{ -it }", "[3, 2, 1]"),
    ("[3, 1, 2].max()", "3"),
    ("[3, 1, 2].min()", "1"),
    ("[1, 2, 3].find{it > 1}", "2"),
    ("[1, 2, 3].find{it > 5}", "null"),
    ("[1, 2, 3].any{it > 2}", "true"),
    ("[1, 2, 3].every{it > 0}", "true"),
    ("[1, 2, 3].count{it > 1}", "2"),
    ("[1, 2, 2, 3].unique()", "[1, 2, 3]"),
    ("[1, 2, 3].reverse()", "[3, 2, 1]"),
    ("[1, 2, 3].first()", "1"),
    ("[1, 2, 3].last()", "3"),
    ("[1, 2, 3].join(\"-\")", "1-2-3"),
    ("[1, 2, 3].size()", "3"),
    ("[1, 2, 3].contains(2)", "true"),
    ("[1, 2, 3].indexOf(3)", "2"),
    ("[1, 2, 3][0]", "1"),
    ("[1, 2, 3][-1]", "3"),
    ("[1, 2, 3].take(2)", "[1, 2]"),
    ("[1, 2, 3].drop(1)", "[2, 3]"),
    ("[1, 2, 3].inject(0){ acc, x -> acc + x }", "6"),
    ("[1, 2, 3] + [4]", "[1, 2, 3, 4]"),
    ("[1, 2, 3] - [2]", "[1, 3]"),
    ("[]", "[]"),
    ("[].size()", "0"),
    ("[1, 2, 3].isEmpty()", "false"),
    ("(1..4).collect{it * it}", "[1, 4, 9, 16]"),
    ("(1..4).sum()", "10"),
    ("2 in [1, 2, 3]", "true"),
    ("[[1, 2], [3]].flatten()", "[1, 2, 3]"),
    ("[1, 2, 3].findIndexOf{it == 2}", "1"),
    ("\"a,b,c\".split(\",\").size()", "3"),
    ("\"a,b,c\".tokenize(\",\")", "[a, b, c]"),
    ("\"abc\".reverse()", "cba"),
    ("\"abc\".capitalize()", "Abc"),
    ("\"abc\".size()", "3"),
    ("\"ab\" * 3", "ababab"),
    ("\"abc\" - \"b\"", "ac"),
    ("\"abc\".padLeft(5)", "  abc"),
    ("\"abc\".padLeft(5, \"*\")", "**abc"),
    ("\"abc\".padRight(5, \".\")", "abc.."),
    ("\"abc\".center(7, \"-\")", "--abc--"),
    ("\"12\".toInteger() + 1", "13"),
    ("\"1.5\".toBigDecimal() * 2", "3.0"),
    ("\"Total: ${[1, 2, 3].sum()}\"", "Total: 6"),
    ("'single ${1}'", "single ${1}"),
    ("\"Hello\"[0]", "H"),
    ("\"Hello\"[1..3]", "ell"),
    ("\"abc\".take(2)", "ab"),
    ("\"42\".isNumber()", "true"),
    ("\"abc\".count(\"b\")", "1"),
    ("5 / 2", "2.5"),
    ("7.intdiv(2)", "3"),
    ("2 ** 10", "1024"),
    ("2 ** 0.5", "1.4142135623730951"),
    ("10 % 3", "1"),
    ("1 <=> 2", "-1"),
    ("1 == 1L", "true"),
    ("1.0 == 1", "true"),
    ("null ?: \"default\"", "default"),
    ("[1, 2, 3]?.size()", "3"),
    ("null?.size()", "null"),
    ("3.5.round()", "4"),
    ("3.14159.round(2)", "3.14"),
    ("1 + 1.5", "2.5"),
    ("3 as String", "3"),
    ("\"5\" as Integer", "5"),
    ("(5 / 2).getClass().getSimpleName()", "BigDecimal"),
    ("1.5.getClass().getSimpleName()", "BigDecimal"),
    ("(2 ** 10).getClass().getSimpleName()", "Integer"),
    ("[] ? \"yes\" : \"no\"", "no"),
    ("\"\" ? \"yes\" : \"no\"", "no"),
    ("0 ? \"yes\" : \"no\"", "no"),
    ("[a: 1, b: 2]", "{a=1, b=2}"),
    ("[a: 1, b: 2].a", "1"),
    ("[a: 1, b: 2][\"b\"]", "2"),
    ("[a: 1, b: 2].size()", "2"),
    ("[a: 1, b: 2].collect{k, v -> k + v}", "[a1, b2]"),
    ("[a: 1, b: 2].keySet()", "[a, b]"),
    ("[1, 2, 3].collect{\"#$it\"}", "[#1, #2, #3]"),
    ("\"a\" + 1 + 2", "a12"),
    ("1 + 2 + \"a\"", "3a"),
    ("[1, 2, 3].sum() / 2", "3"),
];

const WITH_FIELDS: &[(&str, &str)] = &[
    ("$F{amount} * 1.18", "1456.7100"),
    ("$F{amount} * 2", "2469.00"),
    ("$F{amount} / 3", "411.50"),
    ("$F{qty} / 2", "1.5"),
    ("$F{qty} * $F{price}", "7.5"),
    ("$F{qty} + $F{amount}", "1237.50"),
    ("$F{price} * 2", "5.0"),
    ("$F{amount} > 1000 ? \"big\" : \"small\"", "big"),
    ("$F{nothing} ?: \"-\"", "-"),
    ("$F{nothing}?.toUpperCase()", "null"),
    ("$F{name}?.toUpperCase()", "ANA PÉREZ"),
    ("$F{name}.split(\" \").collect{it.capitalize()}.join(\" \")", "Ana Pérez"),
    ("\"${$F{name}} owes ${$F{amount}}\"", "ana pérez owes 1234.50"),
    ("$F{name}.size()", "9"),
    ("$F{name}[0..2]", "ana"),
    ("$F{when}.time", "1772706600000"),
    ("$F{qty} ** 2", "9"),
    ("$F{amount}.setScale(1, java.math.RoundingMode.HALF_UP)", "1234.5"),
    ("$F{amount}.intValue()", "1234"),
    ("$F{qty}.toString().padLeft(5, \"0\")", "00003"),
    ("$F{qty} == 3", "true"),
    ("$F{qty} == 3.0", "true"),
    ("$F{price} == 2.5", "true"),
    ("$F{name} == \"ana pérez\"", "true"),
    ("$F{nothing} == null", "true"),
    ("!$F{nothing}", "true"),
    ("$F{qty} as String", "3"),
    ("($F{qty} as String).size()", "1"),
    ("[$F{qty}, $F{price}].sum()", "5.5"),
    ("[$F{qty}, $F{price}].max()", "3"),
    ("$F{amount}.round(0)", "1235"),
    ("Math.max($F{qty}, 10)", "10"),
    ("$F{qty} % 2 == 1 ? \"odd\" : \"even\"", "odd"),
    ("$F{qty}.intdiv(2)", "1"),
    ("\"Qty: \" + $F{qty}", "Qty: 3"),
];

fn check(cases: &[(&str, &str)]) {
    let env = EvalEnv { locale: &EN_US, groovy: true, now: chrono::NaiveDateTime::default() };
    let mut wrong = Vec::new();
    for (src, want) in cases {
        let got = parse_expr(src, true)
            .map_err(|e| e.to_string())
            .and_then(|e| eval(&e, &Row, &env).map_err(|e| e.to_string()))
            .map(|v| if v.is_null() { "null".to_string() } else { format_value(&v, None, &EN_US) });
        if got.as_deref() != Ok(*want) {
            wrong.push(format!("{src}: JasperReports {want:?}, ferroreport {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{} of {} differ:\n{}", wrong.len(), cases.len(), wrong.join("\n"));
}

#[test]
fn groovy_gives_what_jasperreports_gives() {
    check(JASPER);
}

#[test]
fn groovy_with_fields_gives_what_jasperreports_gives() {
    check(WITH_FIELDS);
}

#[test]
fn closures_cannot_escape_or_loop_forever() {
    let env = EvalEnv { locale: &EN_US, groovy: true, now: chrono::NaiveDateTime::default() };
    for src in [
        "{ it }",
        "(1..100000000).sum()",
        "\"x\" * 100000000",
        "[1] * 100000000",
        "(1..999999).collect { (1..999999).sum { it } }",
    ] {
        let e = parse_expr(src, true).unwrap();
        assert!(eval(&e, &Row, &env).is_err(), "{src}");
    }
}
