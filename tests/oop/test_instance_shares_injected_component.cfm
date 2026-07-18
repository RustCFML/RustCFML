<cfscript>
suiteBegin("Instantiation shares an injected component (reference, not deep-copy)");

// A component reference stored in another component's variables scope must be
// SHARED at instantiation, not deep-copied. Before the reference-boundary fix,
// every `new X()` deep-copied the whole graph of every component it referenced
// — so a shared singleton (e.g. the ColdBox/WireBox Controller) was cloned per
// instance (the Controller graph was copied 332× in one ColdBox spec run,
// ~10 GB). Components are reference types in CFML; Lucee/BoxLang never clone a
// referenced component at `new`.

svc = new SharedSingleton();
h1  = new SingletonHolder( svc );
h2  = new SingletonHolder( svc );

// Mutate the ORIGINAL singleton AFTER both holders were constructed.
svc.increment();

// If the holders held independent deep copies (the old behaviour) they'd still
// read count = 0. Sharing the one instance makes the mutation visible.
assert("holder1 sees shared singleton mutation", h1.getSvc().getCount(), 1);
assert("holder2 sees shared singleton mutation", h2.getSvc().getCount(), 1);

// A mutation THROUGH a holder is visible on the original and the sibling.
h1.getSvc().increment();
assert("original sees mutation made via holder", svc.getCount(), 2);
assert("sibling holder sees mutation made via holder", h2.getSvc().getCount(), 2);

// The reference boundary applies ONLY to instantiation. `duplicate()` must
// still deep-copy nested components (Lucee's deep duplicate clones nested CFCs).
box = { svc : new SharedSingleton() };
box.svc.increment(); // original count = 1
dup = duplicate( box );
dup.svc.increment(); // copy count = 2, original untouched
assert("duplicate() still deep-copies nested component (original)", box.svc.getCount(), 1);
assert("duplicate() still deep-copies nested component (independent copy)", dup.svc.getCount(), 2);

suiteEnd();
</cfscript>
