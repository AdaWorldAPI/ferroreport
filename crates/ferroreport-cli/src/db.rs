//! Database connections for the command line and the server: the template's query runs on
//! SQLite, PostgreSQL, MySQL (or MariaDB) or SQL Server (see `ferroreport::sql`).
//!
//! Each query is read on a thread of its own and its rows are handed over as they arrive, so a
//! large result is never in memory whole. Connections are kept for the next query: a subreport
//! runs its query once for every row of the report around it.

use std::sync::{mpsc, Arc, Mutex};

use base64::Engine;
use ferroreport::sql::{Connection, SqlRow, SqlRows, SqlValue};
use ferroreport::{Error, Result};

/// Rows handed over from the thread that reads them.
const BUFFER: usize = 256;

fn stream<F>(produce: F) -> SqlRows<'static>
where
    F: FnOnce(&mpsc::SyncSender<Result<SqlRow>>) -> Result<()> + Send + 'static,
{
    let (tx, rx) = mpsc::sync_channel(BUFFER);
    std::thread::spawn(move || {
        if let Err(e) = produce(&tx) {
            let _ = tx.send(Err(e));
        }
    });
    Box::new(rx.into_iter())
}

fn db_error(what: &str, e: impl std::fmt::Display) -> Error {
    Error::Data(format!("{what}: {e}"))
}

fn base64(bytes: &[u8]) -> serde_json::Value {
    serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn datetime(d: chrono::NaiveDateTime) -> serde_json::Value {
    serde_json::Value::String(d.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
}

/// Connections kept for the next query.
struct Pool<C> {
    idle: Mutex<Vec<C>>,
    open: Box<dyn Fn() -> Result<C> + Send + Sync>,
}

impl<C> Pool<C> {
    fn take(&self) -> Result<C> {
        if let Some(c) = self.idle.lock().map_err(|_| Error::Data("connection pool poisoned".into()))?.pop() {
            return Ok(c);
        }
        (self.open)()
    }

    fn give_back(&self, c: C) {
        if let Ok(mut idle) = self.idle.lock() {
            if idle.len() < 8 {
                idle.push(c);
            }
        }
    }
}

/// Opens a connection from a URL:
/// `sqlite:path/to/file.db`, `postgres://user:password@host:5432/db`,
/// `mysql://user:password@host:3306/db`, `sqlserver://host:1433;databaseName=db;user=u;password=p`
/// (JDBC style, with or without the `jdbc:` that JasperReports users already have).
/// `FERROREPORT_DB_PASSWORD`, when set, gives the password, which keeps it out of the URL.
pub fn connect(url: &str) -> Result<Arc<dyn Connection>> {
    // Two TLS back ends are compiled in (the MySQL driver brings its own): choose one, or rustls
    // stops at the first encrypted connection.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let url = url.trim();
    let url = url.strip_prefix("jdbc:").unwrap_or(url);
    let password = std::env::var("FERROREPORT_DB_PASSWORD").ok().filter(|p| !p.is_empty());
    let scheme = url.split(':').next().unwrap_or_default().to_ascii_lowercase();
    match scheme.as_str() {
        "sqlite" => sqlite(url),
        "postgres" | "postgresql" => postgres_connect(url, password),
        "mysql" | "mariadb" => mysql_connect(url, password),
        "sqlserver" | "mssql" => sqlserver_connect(url, password),
        _ if ["db", "sqlite", "sqlite3"].iter().any(|e| url.ends_with(&format!(".{e}"))) => sqlite(&format!("sqlite:{url}")),
        "oracle" => Err(Error::Unsupported(
            "Oracle needs Oracle's own client library: export the rows to JSON (or JSON Lines) and pass them with -d".into(),
        )),
        _ => Err(Error::Data(format!("--db {url}: say sqlite:file.db, postgres://..., mysql://... or sqlserver://..."))),
    }
}

// ---------------------------------------------------------------------------------------------
// SQLite

struct Sqlite {
    path: String,
}

fn sqlite(url: &str) -> Result<Arc<dyn Connection>> {
    let path = url.trim_start_matches("sqlite:").trim_start_matches("//").to_string();
    // Opened read-only: a report reads.
    rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| db_error(&path, e))?;
    Ok(Arc::new(Sqlite { path }))
}

fn sqlite_value(v: &SqlValue) -> rusqlite::types::Value {
    use rusqlite::types::Value as V;
    match v {
        SqlValue::Null => V::Null,
        SqlValue::Bool(b) => V::Integer(*b as i64),
        SqlValue::Int(i) => V::Integer(*i),
        SqlValue::Float(f) => V::Real(*f),
        SqlValue::Decimal(d) => d.parse::<f64>().map(V::Real).unwrap_or_else(|_| V::Text(d.clone())),
        SqlValue::Text(s) => V::Text(s.clone()),
        SqlValue::DateTime(d) => V::Text(d.format("%Y-%m-%d %H:%M:%S").to_string()),
        SqlValue::Bytes(b) => V::Blob(b.clone()),
    }
}

impl Connection for Sqlite {
    fn query<'a>(&'a self, sql: &str, params: &[SqlValue]) -> Result<SqlRows<'a>> {
        let (path, sql) = (self.path.clone(), sql.to_string());
        let params: Vec<rusqlite::types::Value> = params.iter().map(sqlite_value).collect();
        Ok(stream(move |tx| {
            let conn = rusqlite::Connection::open_with_flags(
                &path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
            )
            .map_err(|e| db_error(&path, e))?;
            let mut stmt = conn.prepare(&sql).map_err(|e| db_error("query", e))?;
            let names: Vec<String> = stmt.column_names().map(str::to_string).collect();
            let mut rows = stmt.query(rusqlite::params_from_iter(params.iter())).map_err(|e| db_error("query", e))?;
            while let Some(row) = rows.next().map_err(|e| db_error("query", e))? {
                let mut out = Vec::with_capacity(names.len());
                for (i, name) in names.iter().enumerate() {
                    use rusqlite::types::ValueRef as R;
                    let v = match row.get_ref(i).map_err(|e| db_error(name, e))? {
                        R::Null => serde_json::Value::Null,
                        R::Integer(n) => n.into(),
                        R::Real(f) => {
                            serde_json::Number::from_f64(f).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null)
                        }
                        R::Text(t) => String::from_utf8_lossy(t).into_owned().into(),
                        R::Blob(b) => base64(b),
                    };
                    out.push((name.clone(), v));
                }
                if tx.send(Ok(out)).is_err() {
                    break;
                }
            }
            Ok(())
        }))
    }
}

// ---------------------------------------------------------------------------------------------
// PostgreSQL

struct Postgres {
    pool: Arc<Pool<postgres::Client>>,
}

fn postgres_connect(url: &str, password: Option<String>) -> Result<Arc<dyn Connection>> {
    // sslmode as libpq reads it: `disable` sends plain text; `prefer` (the default) and
    // `require` encrypt without checking the server's certificate, as psql does;
    // `verify-ca` and `verify-full` check it against the public roots or `sslrootcert`.
    let (url, mode, root) = postgres_tls_options(url);
    let mut config: postgres::Config = url.parse().map_err(|e| db_error("PostgreSQL URL", e))?;
    if let Some(p) = password {
        config.password(p);
    }
    let open: Box<dyn Fn() -> Result<postgres::Client> + Send + Sync> = if mode == "disable" {
        Box::new(move || config.connect(postgres::NoTls).map_err(|e| db_error("PostgreSQL", e)))
    } else {
        let tls = postgres_tls(mode.starts_with("verify"), root.as_deref())?;
        Box::new(move || config.connect(tls.clone()).map_err(|e| db_error("PostgreSQL", e)))
    };
    let first = open()?;
    let pool = Arc::new(Pool { idle: Mutex::new(vec![first]), open });
    Ok(Arc::new(Postgres { pool }))
}

/// Takes `sslmode` and `sslrootcert` out of the URL, which tokio-postgres reads only in part.
fn postgres_tls_options(url: &str) -> (String, String, Option<String>) {
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    let (mut mode, mut root, mut rest) = ("prefer".to_string(), None, Vec::new());
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        match pair.split_once('=') {
            Some(("sslmode", v)) => mode = v.to_ascii_lowercase(),
            Some(("sslrootcert", v)) => root = Some(v.to_string()),
            _ => rest.push(pair),
        }
    }
    // Encrypted connections are asked for; whether the certificate is checked is decided here.
    rest.push(if mode == "disable" {
        "sslmode=disable"
    } else if mode == "prefer" {
        "sslmode=prefer"
    } else {
        "sslmode=require"
    });
    (format!("{base}?{}", rest.join("&")), mode, root)
}

fn postgres_tls(verify: bool, root: Option<&str>) -> Result<tokio_postgres_rustls::MakeRustlsConnect> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| db_error("TLS", e))?;
    let config = if verify {
        let mut roots = rustls::RootCertStore::empty();
        match root {
            Some(path) => {
                use rustls::pki_types::pem::PemObject;
                for cert in rustls::pki_types::CertificateDer::pem_file_iter(path).map_err(|e| db_error(path, e))? {
                    roots.add(cert.map_err(|e| db_error(path, e))?).map_err(|e| db_error(path, e))?;
                }
            }
            None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
        }
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(AnyCertificate(provider))).with_no_client_auth()
    };
    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}

/// `sslmode=prefer` and `require`: the connection is encrypted and its handshake checked,
/// but the server's certificate is taken as it is, as libpq does.
#[derive(Debug)]
struct AnyCertificate(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// A null of whatever type the server expects.
#[derive(Debug)]
struct AnyNull;

impl postgres::types::ToSql for AnyNull {
    fn to_sql(
        &self,
        _: &postgres::types::Type,
        _: &mut postgres::types::private::BytesMut,
    ) -> std::result::Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        Ok(postgres::types::IsNull::Yes)
    }
    fn accepts(_: &postgres::types::Type) -> bool {
        true
    }
    postgres::types::to_sql_checked!();
}

/// A parameter in the type the server says the query expects.
fn postgres_param(v: &SqlValue, ty: &postgres::types::Type) -> Result<Box<dyn postgres::types::ToSql + Sync + Send>> {
    use postgres::types::Type;
    let text = || match v {
        SqlValue::Text(s) | SqlValue::Decimal(s) => s.clone(),
        SqlValue::Int(i) => i.to_string(),
        SqlValue::Float(f) => f.to_string(),
        SqlValue::Bool(b) => b.to_string(),
        SqlValue::DateTime(d) => d.format("%Y-%m-%d %H:%M:%S").to_string(),
        SqlValue::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
        SqlValue::Null => String::new(),
    };
    let int = || -> Result<i64> {
        match v {
            SqlValue::Int(i) => Ok(*i),
            SqlValue::Bool(b) => Ok(*b as i64),
            SqlValue::Float(f) => Ok(*f as i64),
            _ => text().trim().parse().map_err(|_| Error::Data(format!("query parameter {v:?} is not a whole number"))),
        }
    };
    let float = || -> Result<f64> {
        match v {
            SqlValue::Int(i) => Ok(*i as f64),
            SqlValue::Float(f) => Ok(*f),
            _ => text().trim().parse().map_err(|_| Error::Data(format!("query parameter {v:?} is not a number"))),
        }
    };
    let date = || -> Result<chrono::NaiveDateTime> {
        match v {
            SqlValue::DateTime(d) => Ok(*d),
            _ => ferroreport::data::parse_datetime(&text())
                .ok_or_else(|| Error::Data(format!("query parameter {v:?} is not a date"))),
        }
    };
    if *v == SqlValue::Null {
        return Ok(Box::new(AnyNull));
    }
    Ok(match *ty {
        Type::BOOL => Box::new(match v {
            SqlValue::Bool(b) => *b,
            _ => int()? != 0,
        }),
        Type::INT2 => Box::new(int()? as i16),
        Type::INT4 => Box::new(int()? as i32),
        Type::INT8 => Box::new(int()?),
        Type::OID => Box::new(int()? as u32),
        Type::FLOAT4 => Box::new(float()? as f32),
        Type::FLOAT8 => Box::new(float()?),
        Type::NUMERIC => Box::new(
            text()
                .trim()
                .parse::<rust_decimal::Decimal>()
                .map_err(|_| Error::Data(format!("query parameter {v:?} is not a number")))?,
        ),
        Type::DATE => Box::new(date()?.date()),
        Type::TIMESTAMP => Box::new(date()?),
        Type::TIMESTAMPTZ => Box::new(date()?.and_utc()),
        Type::TIME => Box::new(date()?.time()),
        Type::BYTEA => Box::new(match v {
            SqlValue::Bytes(b) => b.clone(),
            _ => text().into_bytes(),
        }),
        _ => Box::new(text()),
    })
}

fn postgres_cell(row: &postgres::Row, i: usize) -> Result<serde_json::Value> {
    use postgres::types::Type;
    let col = &row.columns()[i];
    let bad = |e: postgres::Error| db_error(col.name(), e);
    let json = |v: Option<serde_json::Value>| v.unwrap_or(serde_json::Value::Null);
    Ok(match *col.type_() {
        Type::BOOL => json(row.try_get::<_, Option<bool>>(i).map_err(bad)?.map(Into::into)),
        Type::INT2 => json(row.try_get::<_, Option<i16>>(i).map_err(bad)?.map(Into::into)),
        Type::INT4 => json(row.try_get::<_, Option<i32>>(i).map_err(bad)?.map(Into::into)),
        Type::INT8 => json(row.try_get::<_, Option<i64>>(i).map_err(bad)?.map(Into::into)),
        Type::OID => json(row.try_get::<_, Option<u32>>(i).map_err(bad)?.map(Into::into)),
        Type::FLOAT4 => json(
            row.try_get::<_, Option<f32>>(i).map_err(bad)?.and_then(|f| serde_json::Number::from_f64(f.into())).map(Into::into),
        ),
        Type::FLOAT8 => {
            json(row.try_get::<_, Option<f64>>(i).map_err(bad)?.and_then(serde_json::Number::from_f64).map(Into::into))
        }
        Type::NUMERIC => json(row.try_get::<_, Option<rust_decimal::Decimal>>(i).map_err(bad)?.map(|d| d.to_string().into())),
        Type::DATE => {
            json(row.try_get::<_, Option<chrono::NaiveDate>>(i).map_err(bad)?.map(|d| d.format("%Y-%m-%d").to_string().into()))
        }
        Type::TIMESTAMP => json(row.try_get::<_, Option<chrono::NaiveDateTime>>(i).map_err(bad)?.map(datetime)),
        Type::TIMESTAMPTZ => {
            json(row.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(i).map_err(bad)?.map(|d| datetime(d.naive_utc())))
        }
        Type::TIME => json(
            row.try_get::<_, Option<chrono::NaiveTime>>(i)
                .map_err(bad)?
                .map(|t| format!("1970-01-01T{}", t.format("%H:%M:%S%.f")).into()),
        ),
        Type::BYTEA => json(row.try_get::<_, Option<Vec<u8>>>(i).map_err(bad)?.map(|b| base64(&b))),
        _ => match row.try_get::<_, Option<String>>(i) {
            Ok(s) => json(s.map(Into::into)),
            Err(_) => {
                return Err(Error::Unsupported(format!(
                    "column `{}`: PostgreSQL type {} (cast it to text in the query: {}::text)",
                    col.name(),
                    col.type_(),
                    col.name()
                )))
            }
        },
    })
}

impl Connection for Postgres {
    fn query<'a>(&'a self, sql: &str, params: &[SqlValue]) -> Result<SqlRows<'a>> {
        let (pool, sql, params) = (self.pool.clone(), sql.to_string(), params.to_vec());
        Ok(stream(move |tx| {
            let mut client = pool.take()?;
            let result = (|| -> Result<()> {
                let stmt = client.prepare(&sql).map_err(|e| db_error("query", e))?;
                let bound: Vec<Box<dyn postgres::types::ToSql + Sync + Send>> =
                    params.iter().zip(stmt.params()).map(|(v, ty)| postgres_param(v, ty)).collect::<Result<_>>()?;
                let mut rows = client
                    .query_raw(&stmt, bound.iter().map(|b| b.as_ref() as &(dyn postgres::types::ToSql + Sync)))
                    .map_err(|e| db_error("query", e))?;
                use postgres::fallible_iterator::FallibleIterator;
                while let Some(row) = rows.next().map_err(|e| db_error("query", e))? {
                    let mut out = Vec::with_capacity(row.len());
                    for i in 0..row.len() {
                        out.push((row.columns()[i].name().to_string(), postgres_cell(&row, i)?));
                    }
                    if tx.send(Ok(out)).is_err() {
                        return Ok(());
                    }
                }
                Ok(())
            })();
            // A connection that failed may be broken: only good ones go back.
            if result.is_ok() && !client.is_closed() {
                pool.give_back(client);
            }
            result
        }))
    }

    fn placeholder(&self, n: usize) -> String {
        format!("${n}")
    }
}

// ---------------------------------------------------------------------------------------------
// MySQL and MariaDB

struct Mysql {
    pool: Arc<Pool<mysql::Conn>>,
}

fn mysql_connect(url: &str, password: Option<String>) -> Result<Arc<dyn Connection>> {
    let url = url.replacen("mariadb://", "mysql://", 1);
    let opts = mysql::Opts::from_url(&url).map_err(|e| db_error("MySQL URL", e))?;
    let mut builder = mysql::OptsBuilder::from_opts(opts);
    if let Some(p) = password {
        builder = builder.pass(Some(p));
    }
    let open = move || -> Result<mysql::Conn> { mysql::Conn::new(builder.clone()).map_err(|e| db_error("MySQL", e)) };
    let first = open()?;
    Ok(Arc::new(Mysql { pool: Arc::new(Pool { idle: Mutex::new(vec![first]), open: Box::new(open) }) }))
}

fn mysql_param(v: &SqlValue) -> mysql::Value {
    use chrono::{Datelike, Timelike};
    match v {
        SqlValue::Null => mysql::Value::NULL,
        SqlValue::Bool(b) => mysql::Value::Int(*b as i64),
        SqlValue::Int(i) => mysql::Value::Int(*i),
        SqlValue::Float(f) => mysql::Value::Double(*f),
        SqlValue::Decimal(s) | SqlValue::Text(s) => mysql::Value::Bytes(s.clone().into_bytes()),
        SqlValue::DateTime(d) => mysql::Value::Date(
            d.year() as u16,
            d.month() as u8,
            d.day() as u8,
            d.hour() as u8,
            d.minute() as u8,
            d.second() as u8,
            d.nanosecond() / 1000,
        ),
        SqlValue::Bytes(b) => mysql::Value::Bytes(b.clone()),
    }
}

/// Fractions of a second as chrono writes them (`%.f`): nothing when there are none.
fn micros(us: u32) -> String {
    if us == 0 {
        String::new()
    } else {
        format!(".{us:06}")
    }
}

fn mysql_cell(column: &mysql::Column, value: &mysql::Value) -> serde_json::Value {
    use mysql::consts::ColumnType as T;
    match value {
        mysql::Value::NULL => serde_json::Value::Null,
        mysql::Value::Int(i) => (*i).into(),
        mysql::Value::UInt(u) => (*u).into(),
        mysql::Value::Float(f) => serde_json::Number::from_f64((*f).into()).map(Into::into).unwrap_or(serde_json::Value::Null),
        mysql::Value::Double(f) => serde_json::Number::from_f64(*f).map(Into::into).unwrap_or(serde_json::Value::Null),
        mysql::Value::Date(y, mo, d, h, mi, s, us) => {
            if column.column_type() == T::MYSQL_TYPE_DATE {
                format!("{y:04}-{mo:02}-{d:02}").into()
            } else {
                format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}{}", micros(*us)).into()
            }
        }
        mysql::Value::Time(_, _, h, mi, s, us) => format!("1970-01-01T{h:02}:{mi:02}:{s:02}{}", micros(*us)).into(),
        mysql::Value::Bytes(b) => {
            // Binary columns (character set 63) hold pictures and the like.
            let binary = column.character_set() == 63
                && matches!(
                    column.column_type(),
                    T::MYSQL_TYPE_BLOB
                        | T::MYSQL_TYPE_TINY_BLOB
                        | T::MYSQL_TYPE_MEDIUM_BLOB
                        | T::MYSQL_TYPE_LONG_BLOB
                        | T::MYSQL_TYPE_VAR_STRING
                        | T::MYSQL_TYPE_STRING
                );
            if binary {
                base64(b)
            } else {
                String::from_utf8_lossy(b).into_owned().into()
            }
        }
    }
}

impl Connection for Mysql {
    fn query<'a>(&'a self, sql: &str, params: &[SqlValue]) -> Result<SqlRows<'a>> {
        let (pool, sql) = (self.pool.clone(), sql.to_string());
        let params: Vec<mysql::Value> = params.iter().map(mysql_param).collect();
        Ok(stream(move |tx| {
            use mysql::prelude::Queryable;
            let mut conn = pool.take()?;
            let result = (|| -> Result<()> {
                let params = if params.is_empty() { mysql::Params::Empty } else { mysql::Params::Positional(params) };
                let mut result = conn.exec_iter(sql.as_str(), params).map_err(|e| db_error("query", e))?;
                let columns = result.columns();
                let columns: Vec<mysql::Column> = columns.as_ref().to_vec();
                for row in result.by_ref() {
                    let row = row.map_err(|e| db_error("query", e))?;
                    let mut out = Vec::with_capacity(columns.len());
                    for (i, c) in columns.iter().enumerate() {
                        let v = row.as_ref(i).map(|v| mysql_cell(c, v)).unwrap_or(serde_json::Value::Null);
                        out.push((c.name_str().into_owned(), v));
                    }
                    if tx.send(Ok(out)).is_err() {
                        break;
                    }
                }
                Ok(())
            })();
            if result.is_ok() {
                pool.give_back(conn);
            }
            result
        }))
    }
}

// ---------------------------------------------------------------------------------------------
// SQL Server

type TdsClient = tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>;

struct SqlServer {
    pool: Arc<Pool<(tokio::runtime::Runtime, TdsClient)>>,
}

fn sqlserver_connect(url: &str, password: Option<String>) -> Result<Arc<dyn Connection>> {
    // JDBC style (`sqlserver://host:1433;databaseName=db;user=u;password=p`), as JasperReports
    // projects write it; `mssql://` the same.
    let jdbc = format!("jdbc:{}", url.replacen("mssql://", "sqlserver://", 1));
    let mut config = tiberius::Config::from_jdbc_string(&jdbc).map_err(|e| db_error("SQL Server URL", e))?;
    if let Some(p) = password {
        let user = jdbc
            .split(';')
            .find_map(|kv| kv.split_once('=').filter(|(k, _)| k.trim().eq_ignore_ascii_case("user")).map(|(_, v)| v.to_string()))
            .unwrap_or_default();
        config.authentication(tiberius::AuthMethod::sql_server(user, p));
    }
    let open = move || -> Result<(tokio::runtime::Runtime, TdsClient)> {
        use tokio_util::compat::TokioAsyncWriteCompatExt;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| db_error("SQL Server", e))?;
        let config = config.clone();
        let client = rt.block_on(async move {
            let tcp = tokio::net::TcpStream::connect(config.get_addr()).await.map_err(|e| db_error("SQL Server", e))?;
            tcp.set_nodelay(true).map_err(|e| db_error("SQL Server", e))?;
            tiberius::Client::connect(config, tcp.compat_write()).await.map_err(|e| db_error("SQL Server", e))
        })?;
        Ok((rt, client))
    };
    let first = open()?;
    Ok(Arc::new(SqlServer { pool: Arc::new(Pool { idle: Mutex::new(vec![first]), open: Box::new(open) }) }))
}

fn sqlserver_cell(data: &tiberius::ColumnData<'static>) -> Result<serde_json::Value> {
    use tiberius::{ColumnData as D, FromSql};
    let json = |v: Option<serde_json::Value>| v.unwrap_or(serde_json::Value::Null);
    let bad = |e: tiberius::error::Error| db_error("column", e);
    Ok(match data {
        D::U8(v) => json(v.map(Into::into)),
        D::I16(v) => json(v.map(Into::into)),
        D::I32(v) => json(v.map(Into::into)),
        D::I64(v) => json(v.map(Into::into)),
        D::F32(v) => json(v.and_then(|f| serde_json::Number::from_f64(f.into())).map(Into::into)),
        D::F64(v) => json(v.and_then(serde_json::Number::from_f64).map(Into::into)),
        D::Bit(v) => json(v.map(Into::into)),
        D::String(v) => json(v.as_ref().map(|s| s.to_string().into())),
        D::Guid(v) => json(v.map(|g| g.to_string().into())),
        D::Binary(v) => json(v.as_ref().map(|b| base64(b))),
        D::Numeric(_) => json(rust_decimal::Decimal::from_sql(data).map_err(bad)?.map(|d| d.to_string().into())),
        D::Xml(v) => json(v.as_ref().map(|x| x.to_string().into())),
        D::DateTime(_) | D::SmallDateTime(_) | D::DateTime2(_) => {
            json(chrono::NaiveDateTime::from_sql(data).map_err(bad)?.map(datetime))
        }
        D::Date(_) => json(chrono::NaiveDate::from_sql(data).map_err(bad)?.map(|d| d.format("%Y-%m-%d").to_string().into())),
        D::Time(_) => json(
            chrono::NaiveTime::from_sql(data).map_err(bad)?.map(|t| format!("1970-01-01T{}", t.format("%H:%M:%S%.f")).into()),
        ),
        D::DateTimeOffset(_) => {
            json(chrono::DateTime::<chrono::FixedOffset>::from_sql(data).map_err(bad)?.map(|d| datetime(d.naive_local())))
        }
    })
}

impl Connection for SqlServer {
    fn query<'a>(&'a self, sql: &str, params: &[SqlValue]) -> Result<SqlRows<'a>> {
        let (pool, sql, params) = (self.pool.clone(), sql.to_string(), params.to_vec());
        Ok(stream(move |tx| {
            let (rt, mut client) = pool.take()?;
            let result = rt.block_on(async {
                use futures_util::TryStreamExt;
                let mut query = tiberius::Query::new(sql.as_str());
                for p in &params {
                    match p {
                        SqlValue::Null => query.bind(Option::<&str>::None),
                        SqlValue::Bool(b) => query.bind(*b),
                        SqlValue::Int(i) => query.bind(*i),
                        SqlValue::Float(f) => query.bind(*f),
                        SqlValue::Decimal(d) => match d.parse::<f64>() {
                            Ok(f) => query.bind(f),
                            Err(_) => query.bind(d.clone()),
                        },
                        SqlValue::Text(s) => query.bind(s.clone()),
                        SqlValue::DateTime(d) => query.bind(*d),
                        SqlValue::Bytes(b) => query.bind(b.clone()),
                    }
                }
                let mut stream = query.query(&mut client).await.map_err(|e| db_error("query", e))?;
                let mut names: Vec<String> = Vec::new();
                while let Some(item) = stream.try_next().await.map_err(|e| db_error("query", e))? {
                    match item {
                        tiberius::QueryItem::Metadata(meta) => {
                            names = meta.columns().iter().map(|c| c.name().to_string()).collect();
                        }
                        tiberius::QueryItem::Row(row) => {
                            let mut out = Vec::with_capacity(names.len());
                            for (i, cell) in row.into_iter().enumerate() {
                                out.push((names.get(i).cloned().unwrap_or_default(), sqlserver_cell(&cell)?));
                            }
                            if tx.send(Ok(out)).is_err() {
                                return Ok(());
                            }
                        }
                    }
                }
                Ok(())
            });
            if result.is_ok() {
                pool.give_back((rt, client));
            }
            result
        }))
    }

    fn placeholder(&self, n: usize) -> String {
        format!("@P{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sqlite_rows_with_their_types() {
        let path = std::env::temp_dir().join(format!("ferroreport-db-{}.sqlite", std::process::id()));
        {
            let c = rusqlite::Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE t (id INTEGER, name TEXT, price REAL, born TEXT, pic BLOB);
                 INSERT INTO t VALUES (1, 'Ana', 12.5, '2026-03-05', x'0102'), (2, 'Luis', NULL, '2026-04-01 10:30:00', NULL);",
            )
            .unwrap();
        }
        let conn = connect(&format!("sqlite:{}", path.display())).unwrap();
        let rows: Vec<SqlRow> =
            conn.query("SELECT * FROM t WHERE id >= ? ORDER BY id", &[SqlValue::Int(1)]).unwrap().collect::<Result<_>>().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][1], ("name".to_string(), serde_json::json!("Ana")));
        assert_eq!(rows[0][4], ("pic".to_string(), serde_json::json!("AQI=")));
        assert_eq!(rows[1][2].1, serde_json::Value::Null);
        // A wrong query is an error, not an empty report.
        let mut bad = conn.query("SELECT nothing FROM nowhere", &[]).unwrap();
        assert!(bad.next().unwrap().is_err());
        std::fs::remove_file(&path).ok();
    }

    /// Runs statements, then a query whose first row must hold `expect` (column -> value).
    /// Set FERROREPORT_TEST_POSTGRES, _MYSQL or _MSSQL to a database URL to run these (the CI
    /// job does, with each server in a container).
    fn check_types(env: &str, setup: &[&str], query: &str, params: &[SqlValue], expect: serde_json::Value) {
        let Ok(url) = std::env::var(env) else {
            eprintln!("{env} not set: skipped");
            return;
        };
        let conn = connect(&url).unwrap();
        for statement in setup {
            conn.query(statement, &[]).unwrap().for_each(|r| {
                r.unwrap();
            });
        }
        let rows: Vec<SqlRow> = conn.query(query, params).unwrap().collect::<Result<_>>().unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        let got: serde_json::Map<String, serde_json::Value> = rows[0].iter().cloned().collect();
        for (name, want) in expect.as_object().unwrap() {
            assert_eq!(got.get(name), Some(want), "column {name}: {got:?}");
        }
        // Nested queries (a subreport per row) each get a connection of their own.
        let mut outer = conn.query(query, params).unwrap();
        let inner: Vec<SqlRow> = conn.query(query, params).unwrap().collect::<Result<_>>().unwrap();
        assert_eq!(inner.len(), 1);
        assert!(outer.next().unwrap().is_ok());
    }

    fn when() -> SqlValue {
        SqlValue::DateTime(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap())
    }

    fn expected() -> serde_json::Value {
        serde_json::json!({
            "i": 7, "big": 9000000000_i64, "n": "1234.50", "f": 0.25, "d": "2026-03-05",
            "ts": "2026-03-05T10:30:00", "t": "1970-01-01T08:15:00", "s": "ñandú", "bin": "AQI=", "nul": null
        })
    }

    #[test]
    fn postgres_types() {
        check_types(
            "FERROREPORT_TEST_POSTGRES",
            &[
                "DROP TABLE IF EXISTS fr_types",
                "CREATE TABLE fr_types (i int4, big int8, n numeric(12,2), f float8, d date, ts timestamp, t time, s text, ok bool, bin bytea, nul int4)",
                "INSERT INTO fr_types VALUES (7, 9000000000, 1234.50, 0.25, '2026-03-05', '2026-03-05 10:30:00', '08:15:00', 'ñandú', true, '\\x0102', NULL)",
            ],
            "SELECT * FROM fr_types WHERE i = $1 AND n >= $2 AND d >= $3 AND s = $4 AND ok = $5",
            &[SqlValue::Int(7), SqlValue::Decimal("1000".into()), when(), SqlValue::Text("ñandú".into()), SqlValue::Bool(true)],
            {
                let mut e = expected();
                e["ok"] = true.into();
                e
            },
        );
    }

    #[test]
    fn mysql_types() {
        check_types(
            "FERROREPORT_TEST_MYSQL",
            &[
                "DROP TABLE IF EXISTS fr_types",
                "CREATE TABLE fr_types (i INT, big BIGINT, n DECIMAL(12,2), f DOUBLE, d DATE, ts DATETIME, t TIME, s VARCHAR(20) CHARACTER SET utf8mb4, ok BOOLEAN, bin BLOB, nul INT)",
                "INSERT INTO fr_types VALUES (7, 9000000000, 1234.50, 0.25, '2026-03-05', '2026-03-05 10:30:00', '08:15:00', 'ñandú', true, x'0102', NULL)",
            ],
            "SELECT * FROM fr_types WHERE i = ? AND n >= ? AND d >= ? AND s = ? AND ok = ?",
            &[SqlValue::Int(7), SqlValue::Decimal("1000".into()), when(), SqlValue::Text("ñandú".into()), SqlValue::Bool(true)],
            {
                let mut e = expected();
                e["ok"] = 1.into();
                e
            },
        );
    }

    #[test]
    fn sqlserver_types() {
        check_types(
            "FERROREPORT_TEST_MSSQL",
            &[
                "IF OBJECT_ID('fr_types') IS NOT NULL DROP TABLE fr_types",
                "CREATE TABLE fr_types (i INT, big BIGINT, n DECIMAL(12,2), f FLOAT, d DATE, ts DATETIME2, t TIME, s NVARCHAR(20), ok BIT, bin VARBINARY(10), nul INT)",
                "INSERT INTO fr_types VALUES (7, 9000000000, 1234.50, 0.25, '2026-03-05', '2026-03-05 10:30:00', '08:15:00', N'ñandú', 1, 0x0102, NULL)",
            ],
            "SELECT * FROM fr_types WHERE i = @P1 AND n >= @P2 AND d >= @P3 AND s = @P4 AND ok = @P5",
            &[SqlValue::Int(7), SqlValue::Decimal("1000".into()), when(), SqlValue::Text("ñandú".into()), SqlValue::Bool(true)],
            {
                let mut e = expected();
                e["ok"] = true.into();
                e
            },
        );
    }

    #[test]
    fn says_which_urls_it_takes() {
        assert!(connect("oracle:thin:@host:1521:db").is_err());
        assert!(connect("ftp://x").is_err());
    }
}
