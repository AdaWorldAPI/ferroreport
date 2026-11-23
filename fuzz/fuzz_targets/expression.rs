//! Any text as a Java or Groovy expression, evaluated with fields, parameters and variables
//! that all exist: errors are fine, panics and endless loops are not.
#![no_main]
use ferroreport::expr::{eval, parse_expr, EvalEnv, Scope};
use ferroreport::value::Value;
use libfuzzer_sys::fuzz_target;

struct Anything;

impl Scope for Anything {
    fn field(&self, name: &str) -> ferroreport::Result<Value> {
        Ok(match name.len() % 5 {
            0 => Value::Null,
            1 => Value::Int(name.len() as i32),
            2 => Value::str(name),
            3 => Value::Double(1.5),
            _ => Value::Json(std::sync::Arc::new(serde_json::json!([{"a": 1}, {"a": "x"}]))),
        })
    }
    fn param(&self, name: &str) -> ferroreport::Result<Value> {
        self.field(name)
    }
    fn variable(&self, name: &str) -> ferroreport::Result<Value> {
        self.field(name)
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&flag, rest)) = data.split_first() else { return };
    let Ok(src) = std::str::from_utf8(rest) else { return };
    let groovy = flag & 1 == 1;
    let Ok(expr) = parse_expr(src, groovy) else { return };
    let env = EvalEnv { locale: &ferroreport::format::locale::EN_US, groovy, now: chrono::NaiveDateTime::default() };
    let _ = eval(&expr, &Anything, &env);
});
