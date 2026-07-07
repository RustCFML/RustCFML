// Fixture: a chain of component methods where the deepest one triggers an
// "undefined member" error. Used by test_error_context.cfm to prove that an
// error thrown several method-frames down and caught by the caller reports the
// full throw-site chain in its tagContext (not just the shallow catch frame).
// This mirrors the shape Wheels hits: controller.redirectTo -> inherited
// URLFor -> a missing struct-key read.
component {
    public any function a() {
        return b();
    }
    public any function b() {
        return c();
    }
    public any function c() {
        var s = {present = 1};
        return s.missingKey; // undefined member — throws deep in the chain
    }
}
