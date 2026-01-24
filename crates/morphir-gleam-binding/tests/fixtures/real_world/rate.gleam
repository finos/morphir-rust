// Adapted from morphir-examples/src/Morphir/Sample/Apps/Shared/Rate.elm
// Union type with variants (some with data, one without)

pub type Rate {
  Fee(Float)
  Rebate(Float)
  GC
}
