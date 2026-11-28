//! A module built against tier 4, running on an engine that only provides
//! tier 3.
//!
//! The load-time gate (`adopt` refusing a module that DECLARES a newer tier) is
//! covered in `foreign_cfml_calls.rs`. This is the other half: a module that
//! declares tier 3 but reaches for a tier-4 call at run time must get a clean
//! answer from the wrapper — never a jump through a vtable slot the host did
//! not fill in — and its failures must behave exactly as they did on tier 3.
//!
//! The older host is simulated by handing the wrapper a copy of the host table
//! cut back to its tier-3 size. That is the whole difference an older engine
//! presents to a module: a smaller `size` and a lower `tier`. It needs its own
//! test binary, because the wrapper's installed table is process-wide.

use std::sync::Mutex;

use cfml_common::dynamic::CfmlValue;
use cfml_module_abi as abi;
use cfml_vm::foreign;
use rustcfml_module::{module, Ctx, Result, Value};

static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Reaches for every tier-4 capability and records what each one said.
fn probe<'a>(ctx: &'a Ctx, args: &[Value<'a>]) -> Result<Value<'a>> {
    let not_a_function = args[0];
    let mut seen = SEEN.lock().unwrap();
    seen.push(match not_a_function.call_as_fn_named(ctx.strukt()) {
        Ok(_) => "named: called".to_string(),
        Err(e) => format!("named: {}", e.message),
    });
    seen.push(format!("take before: {}", ctx.take_error().is_none()));
    // Fails inside the host, which stages "cannot call a value of type …".
    match not_a_function.try_call_as_fn(&[]) {
        Ok(_) => seen.push("try: called".to_string()),
        Err(e) => seen.push(format!("try: {}", e.message)),
    }
    seen.push(format!("take after: {}", ctx.take_error().is_none()));
    drop(seen);
    // Returns normally: on tier 3 nothing could take the staged error, so it
    // must still be raised — the old behaviour, unchanged.
    Ok(ctx.string("returned normally"))
}

module! {
    name: "oldhost",
    version: "1.0.0",
    tier: rustcfml_module::abi::tier::EXECUTION,
    bifs: { "ohProbe" => probe },
}

#[test]
fn tier_4_calls_degrade_cleanly_on_a_tier_3_host() {
    let m = unsafe { foreign::adopt(rustcfml_module_decl(), "oldhost", CfmlValue::Null) }
        .expect("a tier-3 module loads");

    // The host table as a tier-3 engine presents it: everything up to and
    // including `include_template`, and `tier` 3. `adopt` installed the real
    // one; replace it.
    let mut older: abi::HostVtable = unsafe { std::ptr::read(&foreign::HOST_VTABLE) };
    older.size = core::mem::offset_of!(abi::HostVtable, error_take);
    older.tier = abi::tier::EXECUTION;
    let older: &'static abi::HostVtable = Box::leak(Box::new(older));
    unsafe { rustcfml_module::install_host(older) };

    let err = m.bifs[0]
        .call(vec![CfmlValue::string("not a function")])
        .expect_err("the staged error must still be raised, as on tier 3");
    assert!(err.message.contains("cannot call a value of type"), "{}", err.message);

    let seen = SEEN.lock().unwrap().clone();
    assert_eq!(seen.len(), 4, "{seen:?}");
    assert!(
        seen[0].contains("needs an engine that provides extension capability tier 4")
            && seen[0].contains("this one provides tier 3"),
        "{seen:?}"
    );
    assert_eq!(seen[1], "take before: true");
    assert_eq!(seen[2], "try: [function value] failed", "the tier-3 message: {seen:?}");
    assert_eq!(seen[3], "take after: true", "nothing can be taken on tier 3: {seen:?}");
}
