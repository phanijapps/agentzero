//! sqlite-vec extension loader.
//!
//! Called on every new connection opened against `knowledge.db`.
//! Loads the bundled sqlite-vec shared library so `vec0` virtual tables
//! and `vec_distance_cosine()` are available.
//!
//! # API note
//!
//! The `sqlite-vec` crate (0.1.9) declares `sqlite3_vec_init` as a C extern
//! with no Rust-visible arguments, mirroring how SQLite extension init
//! functions are registered via `sqlite3_auto_extension`.  The real C
//! signature is the standard extension entry-point:
//!
//! ```c
//! int sqlite3_vec_init(sqlite3 *db, char **pzErrMsg,
//!                      const sqlite3_api_routines *pApi);
//! ```
//!
//! We `transmute` the function pointer to `rusqlite::auto_extension::RawAutoExtension`
//! and invoke it directly on the connection handle, so the extension is
//! loaded into exactly one connection rather than every future connection
//! (which `sqlite3_auto_extension` would do).

use rusqlite::{auto_extension::RawAutoExtension, ffi, Connection, Error as RusqliteError, Result};

/// Load the sqlite-vec extension into the given connection.
///
/// Returns an error if the extension cannot be loaded. Callers should
/// fail daemon startup rather than continue — sqlite-vec is not optional
/// in memory v2.
pub fn load_sqlite_vec(conn: &Connection) -> Result<()> {
    // SAFETY:
    //  1. `sqlite3_vec_init` has C signature matching `RawAutoExtension`.
    //     The Rust binding omits the arguments but the symbols are identical.
    //  2. `conn.handle()` is valid for the lifetime of `conn`.
    //  3. We pass a null pzErrMsg — the return code is sufficient for error
    //     detection; if needed SQLite also sets the connection error.
    unsafe {
        let init_fn: RawAutoExtension =
            std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());

        let mut err_msg: *mut std::os::raw::c_char = std::ptr::null_mut();
        let rc = init_fn(conn.handle(), &mut err_msg, std::ptr::null());

        if err_msg.is_null() && rc == ffi::SQLITE_OK {
            return Ok(());
        }

        let msg = if !err_msg.is_null() {
            let s = std::ffi::CStr::from_ptr(err_msg)
                .to_string_lossy()
                .into_owned();
            // sqlite3_free the pointer allocated by SQLite.
            ffi::sqlite3_free(err_msg.cast());
            s
        } else {
            format!("sqlite3_vec_init returned code {rc}")
        };

        Err(RusqliteError::SqliteFailure(ffi::Error::new(rc), Some(msg)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_loads_on_in_memory_db() {
        let conn = Connection::open_in_memory().expect("open in-memory");
        load_sqlite_vec(&conn).expect("load sqlite-vec");

        // Smoke test: create a vec0 virtual table.
        conn.execute_batch("CREATE VIRTUAL TABLE t USING vec0(id TEXT PRIMARY KEY, v FLOAT[4]);")
            .expect("create vec0 table");

        // Smoke test: insert and query.
        let vec_str = serde_json::to_string(&[0.1_f32, 0.2, 0.3, 0.4]).unwrap();
        conn.execute(
            "INSERT INTO t(id, v) VALUES ('a', ?1)",
            rusqlite::params![vec_str],
        )
        .expect("insert");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 1);
    }
}
