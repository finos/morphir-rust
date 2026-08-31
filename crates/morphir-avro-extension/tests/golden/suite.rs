use std::{env, ffi::OsStr, fs, path::PathBuf};

use crate::support::projection::{
    alias, customer_record, field, package, reference, value_specification,
};
use apache_avro::{
    Schema,
    schema::{Name, NamesRef, ResolvedSchema},
};
use morphir_avro_extension::{
    Aliases, AvroOptions, Constructor, DistributionKind, EntryPointKind, EntryPointMetadata,
    Projection, ProjectionDependency, ProjectionModule, ProjectionPackage, TypeDeclaration,
    TypeExpr, TypeMapping, Unsupported, ValueKind, generate,
};
use serde_json::Value;

const STRING: &str = "morphir/SDK:string#string";
const SET: &str = "morphir/SDK:set#set";
const RESULT: &str = "morphir/SDK:result#result";
const LOCAL_DATE: &str = "morphir/SDK:local-date#local-date";
const LOCAL_TIME: &str = "morphir/SDK:local-time#local-time";
const INSTANT: &str = "morphir/SDK:instant#instant";
const UUID: &str = "morphir/SDK:uuid#uuid";
const CHAR: &str = "morphir/SDK:char#char";
const MAYBE: &str = "morphir/SDK:maybe#maybe";
const DICT: &str = "morphir/SDK:dict#dict";
const DECIMAL: &str = "morphir/SDK:decimal#decimal";
const CUSTOMER: &str = "Acme:Customer:Customer";

#[derive(Clone)]
pub(crate) struct GoldenCase {
    golden: &'static str,
    expected_path: &'static str,
    package: ProjectionPackage,
    options: AvroOptions,
}

mod assertions;
mod cases;
mod dependencies;
mod golden_io;
mod mothers;
mod registry;

pub(crate) use assertions::*;
pub(crate) use mothers::*;
