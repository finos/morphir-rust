# Independent scoped-update fixture

Exact frozen inputs and expected full locks copied from the parent Morphir corpus,
`spec/package/mck/fixtures/mvp-scoped-update/signed`, authored independently from
this runtime. The parent fixture generator and controller own their provenance.
No expected lock is captured from resolver output.

The old lock has four nodes and expired historical metadata. The current signed
view retains each old immutable record and offers newer target, child and sibling
releases. Eligible update changes eligibility 1.2.0 to 1.3.0 and its required child
1.0.0 to 1.1.0; sibling 1.0.0 and root 1.0.0 stay fixed. Exact targeting 1.2.0 keeps
the old graph. Variant metadata exercises scope conflict, frozen yanked sibling or
root, and revoked sibling. The newer yanked 1.9.0 target is never eligible.
