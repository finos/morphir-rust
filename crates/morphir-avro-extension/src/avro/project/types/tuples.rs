use super::super::encoding::canonical_tuple_identity;
use super::super::*;

impl Projector<'_> {
    pub(super) fn project_tuple(
        &mut self,
        schema_namespace: &str,
        owner_source: &str,
        elements: &[TypeExpr],
    ) -> Result<AvroType, AvroDiagnostic> {
        let projected_elements = elements
            .iter()
            .map(|element| self.project_type(schema_namespace, owner_source, element))
            .collect::<Result<Vec<_>, _>>()?;
        let identity = canonical_tuple_identity(&projected_elements);
        let digest = Sha256::digest(&identity);
        let prefix = digest[..6]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let full_name = AvroFullName::new(schema_namespace.to_owned(), format!("Tuple_{prefix}"))?;
        self.registry
            .claim_bytes(&full_name.to_string(), &identity)?;
        if !self.contains_schema(&full_name) {
            let fields = projected_elements
                .into_iter()
                .enumerate()
                .map(|(index, element)| {
                    AvroField::new(format!("item{}", index + 1), element, Properties::new())
                })
                .collect::<Result<Vec<_>, _>>()?;
            self.insert(
                NamedSchema::Record(RecordSchema::new(
                    full_name.clone(),
                    fields,
                    None,
                    BTreeMap::from([("morphir.type".to_owned(), json!("Tuple"))]),
                )?),
                self.ownership_for_source(owner_source),
            );
        }
        Ok(AvroType::Named(full_name))
    }
}
