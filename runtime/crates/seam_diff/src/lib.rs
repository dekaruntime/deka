use std::collections::{BTreeMap, BTreeSet};

use seam_ir::{SeamContract, SeamDefinition, SeamEnum, SeamEnumVariant, SeamRecord, SeamType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compat {
    Identical,
    Additive,
    Breaking { reasons: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeamErrorKind {
    UnknownField,
    MissingVariant,
    TypeMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeamError {
    pub kind: SeamErrorKind,
    pub contract: String,
    pub offending: String,
    pub message: String,
    pub producer_location: Option<SourceLocation>,
    pub consumer_location: Option<SourceLocation>,
}

pub fn diff(old: &SeamContract, new: &SeamContract) -> Compat {
    if old == new {
        return Compat::Identical;
    }

    let mut breaking = Vec::new();

    if old.format != new.format {
        breaking.push(format!(
            "contract format changed from {} to {}",
            old.format, new.format
        ));
    }
    if old.name != new.name {
        breaking.push(format!(
            "contract name changed from {} to {}",
            old.name, new.name
        ));
    }
    if old.boundaries != new.boundaries {
        breaking.push("contract boundaries changed".to_string());
    }

    let old_defs = definitions_by_name(old);
    let new_defs = definitions_by_name(new);

    for (name, old_def) in &old_defs {
        match new_defs.get(name) {
            Some(new_def) => compare_definition(name, old_def, new_def, &mut breaking),
            None => breaking.push(format!("definition {} was removed", name)),
        }
    }

    if breaking.is_empty() {
        Compat::Additive
    } else {
        Compat::Breaking { reasons: breaking }
    }
}

pub fn check_consumer(contract: &SeamContract, consumer: &SeamContract) -> Vec<SeamError> {
    let contract_defs = definitions_by_name(contract);
    let consumer_defs = definitions_by_name(consumer);
    let mut errors = Vec::new();

    for (name, consumer_def) in consumer_defs {
        let Some(contract_def) = contract_defs.get(&name) else {
            continue;
        };

        match (contract_def, consumer_def) {
            (SeamDefinition::Record(contract_record), SeamDefinition::Record(consumer_record)) => {
                check_record_consumer(contract, contract_record, consumer_record, &mut errors);
            }
            (SeamDefinition::Enum(contract_enum), SeamDefinition::Enum(consumer_enum)) => {
                check_enum_consumer(contract, contract_enum, consumer_enum, &mut errors);
            }
            _ => errors.push(error(
                SeamErrorKind::TypeMismatch,
                contract,
                name,
                format!("type mismatch for definition {}", name),
            )),
        }
    }

    errors
}

fn definitions_by_name(contract: &SeamContract) -> BTreeMap<&str, &SeamDefinition> {
    contract
        .definitions
        .iter()
        .map(|definition| (definition_name(definition), definition))
        .collect()
}

fn definition_name(definition: &SeamDefinition) -> &str {
    match definition {
        SeamDefinition::Record(record) => &record.name,
        SeamDefinition::Enum(enumeration) => &enumeration.name,
    }
}

fn compare_definition(
    name: &str,
    old: &SeamDefinition,
    new: &SeamDefinition,
    breaking: &mut Vec<String>,
) {
    match (old, new) {
        (SeamDefinition::Record(old_record), SeamDefinition::Record(new_record)) => {
            compare_fields(name, &old_record.fields, &new_record.fields, breaking);
        }
        (SeamDefinition::Enum(old_enum), SeamDefinition::Enum(new_enum)) => {
            compare_enum(name, old_enum, new_enum, breaking);
        }
        _ => breaking.push(format!("definition {} changed kind", name)),
    }
}

fn compare_enum(name: &str, old: &SeamEnum, new: &SeamEnum, breaking: &mut Vec<String>) {
    let old_variants = variants_by_name(old);
    let new_variants = variants_by_name(new);

    for (variant_name, old_variant) in &old_variants {
        match new_variants.get(variant_name) {
            Some(new_variant) => compare_fields(
                &format!("{}.{}", name, variant_name),
                &old_variant.fields,
                &new_variant.fields,
                breaking,
            ),
            None => breaking.push(format!("enum {} removed variant {}", name, variant_name)),
        }
    }
}

fn variants_by_name(enumeration: &SeamEnum) -> BTreeMap<&str, &SeamEnumVariant> {
    enumeration
        .variants
        .iter()
        .map(|variant| (variant.name.as_str(), variant))
        .collect()
}

fn compare_fields(
    owner: &str,
    old_fields: &BTreeMap<String, SeamType>,
    new_fields: &BTreeMap<String, SeamType>,
    breaking: &mut Vec<String>,
) {
    for (field_name, old_type) in old_fields {
        match new_fields.get(field_name) {
            Some(new_type) => compare_field(owner, field_name, old_type, new_type, breaking),
            None => breaking.push(format!("{} removed field {}", owner, field_name)),
        }
    }

    for (field_name, new_type) in new_fields {
        if !old_fields.contains_key(field_name) && !is_optional(new_type) {
            breaking.push(format!("{} added required field {}", owner, field_name));
        }
    }
}

fn compare_field(
    owner: &str,
    field_name: &str,
    old_type: &SeamType,
    new_type: &SeamType,
    breaking: &mut Vec<String>,
) {
    match (is_optional(old_type), is_optional(new_type)) {
        (false, true) if option_item(new_type) == Some(old_type) => {}
        (true, false) if option_item(old_type) == Some(new_type) => {
            breaking.push(format!(
                "{}.{} changed from optional to required",
                owner, field_name
            ));
        }
        _ if old_type == new_type => {}
        _ => breaking.push(format!(
            "{}.{} changed type from {} to {}",
            owner,
            field_name,
            render_type(old_type),
            render_type(new_type)
        )),
    }
}

fn check_record_consumer(
    contract: &SeamContract,
    contract_record: &SeamRecord,
    consumer_record: &SeamRecord,
    errors: &mut Vec<SeamError>,
) {
    for (field_name, consumer_type) in &consumer_record.fields {
        let offending = format!("{}.{}", consumer_record.name, field_name);
        match contract_record.fields.get(field_name) {
            Some(contract_type) if contract_type != consumer_type => errors.push(error(
                SeamErrorKind::TypeMismatch,
                contract,
                &offending,
                format!(
                    "type mismatch on field {}: contract has {}, consumer reads {}",
                    offending,
                    render_type(contract_type),
                    render_type(consumer_type)
                ),
            )),
            Some(_) => {}
            None => errors.push(error(
                SeamErrorKind::UnknownField,
                contract,
                &offending,
                format!("reads field {} not in contract", offending),
            )),
        }
    }
}

fn check_enum_consumer(
    contract: &SeamContract,
    contract_enum: &SeamEnum,
    consumer_enum: &SeamEnum,
    errors: &mut Vec<SeamError>,
) {
    let consumer_variants: BTreeSet<&str> = consumer_enum
        .variants
        .iter()
        .map(|variant| variant.name.as_str())
        .collect();

    for variant in &contract_enum.variants {
        if !consumer_variants.contains(variant.name.as_str()) {
            let offending = format!("{}.{}", contract_enum.name, variant.name);
            errors.push(error(
                SeamErrorKind::MissingVariant,
                contract,
                &offending,
                format!("missing variant {}", offending),
            ));
        }
    }

    let contract_variants = variants_by_name(contract_enum);
    for consumer_variant in &consumer_enum.variants {
        let Some(contract_variant) = contract_variants.get(consumer_variant.name.as_str()) else {
            continue;
        };
        check_variant_fields(
            contract,
            contract_enum,
            contract_variant,
            consumer_variant,
            errors,
        );
    }
}

fn check_variant_fields(
    contract: &SeamContract,
    contract_enum: &SeamEnum,
    contract_variant: &SeamEnumVariant,
    consumer_variant: &SeamEnumVariant,
    errors: &mut Vec<SeamError>,
) {
    for (field_name, consumer_type) in &consumer_variant.fields {
        let offending = format!(
            "{}.{}.{}",
            contract_enum.name, consumer_variant.name, field_name
        );
        match contract_variant.fields.get(field_name) {
            Some(contract_type) if contract_type != consumer_type => errors.push(error(
                SeamErrorKind::TypeMismatch,
                contract,
                &offending,
                format!(
                    "type mismatch on field {}: contract has {}, consumer reads {}",
                    offending,
                    render_type(contract_type),
                    render_type(consumer_type)
                ),
            )),
            Some(_) => {}
            None => errors.push(error(
                SeamErrorKind::UnknownField,
                contract,
                &offending,
                format!("reads field {} not in contract", offending),
            )),
        }
    }
}

fn error(
    kind: SeamErrorKind,
    contract: &SeamContract,
    offending: &str,
    message: String,
) -> SeamError {
    SeamError {
        kind,
        contract: contract.name.clone(),
        offending: offending.to_string(),
        message,
        producer_location: None,
        consumer_location: None,
    }
}

fn is_optional(ty: &SeamType) -> bool {
    matches!(ty, SeamType::Option { .. })
}

fn option_item(ty: &SeamType) -> Option<&SeamType> {
    match ty {
        SeamType::Option { item } => Some(item),
        _ => None,
    }
}

fn render_type(ty: &SeamType) -> String {
    match ty {
        SeamType::Primitive { name } => format!("{:?}", name),
        SeamType::Named { name } => name.clone(),
        SeamType::Option { item } => format!("Option<{}>", render_type(item)),
        SeamType::List { item } => format!("List<{}>", render_type(item)),
        SeamType::Map { key, value } => {
            format!("Map<{}, {}>", render_type(key), render_type(value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seam_ir::{SeamEnum, SeamEnumVariant, SeamPrimitive};

    #[test]
    fn identical_contracts_are_identical() {
        let old = base_contract();
        let new = old.clone();

        assert_eq!(diff(&old, &new), Compat::Identical);
    }

    #[test]
    fn adding_optional_field_is_additive() {
        let old = base_contract();
        let mut new = old.clone();
        record_mut(&mut new, "Response")
            .fields
            .insert("subtitle".to_string(), option(string()));

        assert_eq!(diff(&old, &new), Compat::Additive);
    }

    #[test]
    fn adding_enum_variant_is_additive() {
        let old = base_contract();
        let mut new = old.clone();
        enum_mut(&mut new, "Status")
            .variants
            .push(variant("Archived"));

        assert_eq!(diff(&old, &new), Compat::Additive);
    }

    #[test]
    fn required_to_optional_field_is_additive() {
        let old = base_contract();
        let mut new = old.clone();
        record_mut(&mut new, "Response")
            .fields
            .insert("title".to_string(), option(string()));

        assert_eq!(diff(&old, &new), Compat::Additive);
    }

    #[test]
    fn removing_required_field_is_breaking() {
        let mut new = base_contract();
        let old = new.clone();
        record_mut(&mut new, "Response").fields.remove("title");

        assert_breaking_contains(diff(&old, &new), "removed field title");
    }

    #[test]
    fn retyping_required_field_is_breaking() {
        let old = base_contract();
        let mut new = old.clone();
        record_mut(&mut new, "Response")
            .fields
            .insert("title".to_string(), int());

        assert_breaking_contains(diff(&old, &new), "Response.title changed type");
    }

    #[test]
    fn retyping_optional_field_is_breaking() {
        let old = base_contract();
        let mut new = old.clone();
        record_mut(&mut new, "Response")
            .fields
            .insert("summary".to_string(), option(int()));

        assert_breaking_contains(diff(&old, &new), "Response.summary changed type");
    }

    #[test]
    fn removing_enum_variant_is_breaking() {
        let mut new = base_contract();
        let old = new.clone();
        enum_mut(&mut new, "Status")
            .variants
            .retain(|variant| variant.name != "Failed");

        assert_breaking_contains(diff(&old, &new), "removed variant Failed");
    }

    #[test]
    fn optional_to_required_field_is_breaking() {
        let old = base_contract();
        let mut new = old.clone();
        record_mut(&mut new, "Response")
            .fields
            .insert("summary".to_string(), string());

        assert_breaking_contains(
            diff(&old, &new),
            "Response.summary changed from optional to required",
        );
    }

    #[test]
    fn consumer_reading_unknown_field_is_an_error() {
        let contract = base_contract();
        let mut consumer = contract.clone();
        record_mut(&mut consumer, "Response")
            .fields
            .insert("unknown".to_string(), string());

        let errors = check_consumer(&contract, &consumer);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, SeamErrorKind::UnknownField);
        assert_eq!(errors[0].offending, "Response.unknown");
        assert!(errors[0]
            .message
            .contains("reads field Response.unknown not in contract"));
        assert_eq!(errors[0].producer_location, None);
        assert_eq!(errors[0].consumer_location, None);
    }

    #[test]
    fn consumer_enum_match_missing_contract_variant_is_an_error() {
        let contract = base_contract();
        let mut consumer = contract.clone();
        enum_mut(&mut consumer, "Status")
            .variants
            .retain(|variant| variant.name != "Failed");

        let errors = check_consumer(&contract, &consumer);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, SeamErrorKind::MissingVariant);
        assert_eq!(errors[0].offending, "Status.Failed");
        assert!(errors[0].message.contains("missing variant Status.Failed"));
    }

    #[test]
    fn consumer_field_type_mismatch_is_an_error() {
        let contract = base_contract();
        let mut consumer = contract.clone();
        record_mut(&mut consumer, "Response")
            .fields
            .insert("title".to_string(), int());

        let errors = check_consumer(&contract, &consumer);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, SeamErrorKind::TypeMismatch);
        assert_eq!(errors[0].offending, "Response.title");
        assert!(errors[0]
            .message
            .contains("type mismatch on field Response.title"));
    }

    #[test]
    fn consumer_variant_payload_field_checks_match_records() {
        let contract = base_contract();
        let mut consumer = contract.clone();
        enum_mut(&mut consumer, "Status")
            .variants
            .iter_mut()
            .find(|variant| variant.name == "Failed")
            .unwrap()
            .fields
            .insert("debug".to_string(), string());

        let errors = check_consumer(&contract, &consumer);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, SeamErrorKind::UnknownField);
        assert_eq!(errors[0].offending, "Status.Failed.debug");
    }

    fn base_contract() -> SeamContract {
        let mut contract = SeamContract::new("storefront", 1);
        contract
            .definitions
            .push(SeamDefinition::Record(SeamRecord {
                name: "Response".to_string(),
                fields: BTreeMap::from([
                    ("title".to_string(), string()),
                    ("summary".to_string(), option(string())),
                ]),
            }));
        contract.definitions.push(SeamDefinition::Enum(SeamEnum {
            name: "Status".to_string(),
            variants: vec![variant("Ready"), failed_variant()],
        }));
        contract
    }

    fn record_mut<'a>(contract: &'a mut SeamContract, name: &str) -> &'a mut SeamRecord {
        contract
            .definitions
            .iter_mut()
            .find_map(|definition| match definition {
                SeamDefinition::Record(record) if record.name == name => Some(record),
                _ => None,
            })
            .unwrap()
    }

    fn enum_mut<'a>(contract: &'a mut SeamContract, name: &str) -> &'a mut SeamEnum {
        contract
            .definitions
            .iter_mut()
            .find_map(|definition| match definition {
                SeamDefinition::Enum(enumeration) if enumeration.name == name => Some(enumeration),
                _ => None,
            })
            .unwrap()
    }

    fn variant(name: &str) -> SeamEnumVariant {
        SeamEnumVariant {
            name: name.to_string(),
            fields: BTreeMap::new(),
        }
    }

    fn failed_variant() -> SeamEnumVariant {
        SeamEnumVariant {
            name: "Failed".to_string(),
            fields: BTreeMap::from([("reason".to_string(), string())]),
        }
    }

    fn string() -> SeamType {
        SeamType::Primitive {
            name: SeamPrimitive::String,
        }
    }

    fn int() -> SeamType {
        SeamType::Primitive {
            name: SeamPrimitive::Int,
        }
    }

    fn option(item: SeamType) -> SeamType {
        SeamType::Option {
            item: Box::new(item),
        }
    }

    fn assert_breaking_contains(compat: Compat, needle: &str) {
        match compat {
            Compat::Breaking { reasons } => assert!(
                reasons.iter().any(|reason| reason.contains(needle)),
                "missing reason containing {needle:?}; got {reasons:?}"
            ),
            other => panic!("expected breaking compatibility, got {other:?}"),
        }
    }
}
