use std::collections::{BTreeMap, HashMap};

use seam_ir::{
    SeamBoundary, SeamContract, SeamDefinition, SeamEnum, SeamEnumVariant, SeamPrimitive,
    SeamRecord, SeamType,
};
use serde::{Deserialize, Serialize};

pub trait ToSeam {
    fn seam_type() -> SeamType;
    fn seam_definition() -> Option<SeamDefinition> {
        None
    }
    fn seam_definitions() -> Vec<SeamDefinition> {
        Self::seam_definition().into_iter().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorefrontRequest {
    pub url: String,
    pub path: String,
    pub pathname: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorefrontResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
    #[serde(default)]
    pub body_base64: Option<String>,
    #[serde(default)]
    pub upgrade: Option<serde_json::Value>,
}

impl StorefrontResponse {
    pub fn from_value(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value)
    }
}

impl ToSeam for String {
    fn seam_type() -> SeamType {
        primitive(SeamPrimitive::String)
    }
}

impl ToSeam for u16 {
    fn seam_type() -> SeamType {
        primitive(SeamPrimitive::Int)
    }
}

impl ToSeam for u32 {
    fn seam_type() -> SeamType {
        primitive(SeamPrimitive::Int)
    }
}

impl ToSeam for i64 {
    fn seam_type() -> SeamType {
        primitive(SeamPrimitive::Int)
    }
}

impl ToSeam for bool {
    fn seam_type() -> SeamType {
        primitive(SeamPrimitive::Bool)
    }
}

impl<T: ToSeam> ToSeam for Option<T> {
    fn seam_type() -> SeamType {
        SeamType::Option {
            item: Box::new(T::seam_type()),
        }
    }

    fn seam_definitions() -> Vec<SeamDefinition> {
        T::seam_definitions()
    }
}

impl<T: ToSeam> ToSeam for Vec<T> {
    fn seam_type() -> SeamType {
        SeamType::List {
            item: Box::new(T::seam_type()),
        }
    }

    fn seam_definitions() -> Vec<SeamDefinition> {
        T::seam_definitions()
    }
}

impl<K: ToSeam, V: ToSeam> ToSeam for HashMap<K, V> {
    fn seam_type() -> SeamType {
        SeamType::Map {
            key: Box::new(K::seam_type()),
            value: Box::new(V::seam_type()),
        }
    }

    fn seam_definitions() -> Vec<SeamDefinition> {
        let mut definitions = K::seam_definitions();
        definitions.extend(V::seam_definitions());
        definitions
    }
}

impl ToSeam for serde_json::Value {
    fn seam_type() -> SeamType {
        SeamType::Map {
            key: Box::new(primitive(SeamPrimitive::String)),
            value: Box::new(primitive(SeamPrimitive::String)),
        }
    }
}

impl ToSeam for StorefrontRequest {
    fn seam_type() -> SeamType {
        SeamType::Named {
            name: "StorefrontRequest".to_string(),
        }
    }

    fn seam_definition() -> Option<SeamDefinition> {
        let mut fields = BTreeMap::new();
        fields.insert("body".to_string(), Option::<String>::seam_type());
        fields.insert(
            "headers".to_string(),
            HashMap::<String, String>::seam_type(),
        );
        fields.insert("method".to_string(), String::seam_type());
        fields.insert("path".to_string(), String::seam_type());
        fields.insert("pathname".to_string(), String::seam_type());
        fields.insert("url".to_string(), String::seam_type());

        Some(seam_record_definition("StorefrontRequest", fields))
    }
}

impl ToSeam for StorefrontResponse {
    fn seam_type() -> SeamType {
        SeamType::Named {
            name: "StorefrontResponse".to_string(),
        }
    }

    fn seam_definition() -> Option<SeamDefinition> {
        let mut fields = BTreeMap::new();
        fields.insert("body".to_string(), String::seam_type());
        fields.insert("body_base64".to_string(), Option::<String>::seam_type());
        fields.insert(
            "headers".to_string(),
            HashMap::<String, String>::seam_type(),
        );
        fields.insert("status".to_string(), u16::seam_type());
        fields.insert(
            "upgrade".to_string(),
            Option::<serde_json::Value>::seam_type(),
        );

        Some(seam_record_definition("StorefrontResponse", fields))
    }
}

pub fn storefront_contract() -> SeamContract {
    let mut contract = SeamContract::new("storefront", 1);
    contract.boundaries.push(SeamBoundary {
        function: "fetch".to_string(),
        request: "StorefrontRequest".to_string(),
        response: "StorefrontResponse".to_string(),
    });
    contract
        .definitions
        .extend(StorefrontRequest::seam_definitions());
    contract
        .definitions
        .extend(StorefrontResponse::seam_definitions());
    contract
}

fn primitive(name: SeamPrimitive) -> SeamType {
    SeamType::Primitive { name }
}

pub fn seam_record_definition(
    name: impl Into<String>,
    fields: BTreeMap<String, SeamType>,
) -> SeamDefinition {
    SeamDefinition::Record(SeamRecord {
        name: name.into(),
        fields,
    })
}

pub fn seam_enum_definition(
    name: impl Into<String>,
    variants: Vec<SeamEnumVariant>,
) -> SeamDefinition {
    SeamDefinition::Enum(SeamEnum {
        name: name.into(),
        variants,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[allow(dead_code)]
    struct ShippingAddress {
        street: String,
        city: String,
    }

    impl ToSeam for ShippingAddress {
        fn seam_type() -> SeamType {
            SeamType::Named {
                name: "ShippingAddress".to_string(),
            }
        }

        fn seam_definition() -> Option<SeamDefinition> {
            let mut fields = BTreeMap::new();
            fields.insert("city".to_string(), String::seam_type());
            fields.insert("street".to_string(), String::seam_type());

            Some(seam_record_definition("ShippingAddress", fields))
        }
    }

    #[allow(dead_code)]
    struct OrderSummary {
        id: String,
        shipping: ShippingAddress,
    }

    impl ToSeam for OrderSummary {
        fn seam_type() -> SeamType {
            SeamType::Named {
                name: "OrderSummary".to_string(),
            }
        }

        fn seam_definition() -> Option<SeamDefinition> {
            let mut fields = BTreeMap::new();
            fields.insert("id".to_string(), String::seam_type());
            fields.insert("shipping".to_string(), ShippingAddress::seam_type());

            Some(seam_record_definition("OrderSummary", fields))
        }

        fn seam_definitions() -> Vec<SeamDefinition> {
            let mut definitions =
                vec![Self::seam_definition().expect("OrderSummary seam definition")];
            definitions.extend(ShippingAddress::seam_definitions());
            definitions
        }
    }

    #[allow(dead_code)]
    enum FulfillmentKind {
        Digital,
        Physical {
            carrier: String,
            address: ShippingAddress,
        },
        Backorder {
            restock_days: u32,
        },
    }

    impl ToSeam for FulfillmentKind {
        fn seam_type() -> SeamType {
            SeamType::Named {
                name: "FulfillmentKind".to_string(),
            }
        }

        fn seam_definition() -> Option<SeamDefinition> {
            let mut variants = Vec::new();
            variants.push(SeamEnumVariant {
                name: "Digital".to_string(),
                fields: BTreeMap::new(),
            });

            let mut physical_fields = BTreeMap::new();
            physical_fields.insert("address".to_string(), ShippingAddress::seam_type());
            physical_fields.insert("carrier".to_string(), String::seam_type());
            variants.push(SeamEnumVariant {
                name: "Physical".to_string(),
                fields: physical_fields,
            });

            let mut backorder_fields = BTreeMap::new();
            backorder_fields.insert("restock_days".to_string(), u32::seam_type());
            variants.push(SeamEnumVariant {
                name: "Backorder".to_string(),
                fields: backorder_fields,
            });

            Some(seam_enum_definition("FulfillmentKind", variants))
        }

        fn seam_definitions() -> Vec<SeamDefinition> {
            let mut definitions =
                vec![Self::seam_definition().expect("FulfillmentKind seam definition")];
            definitions.extend(ShippingAddress::seam_definitions());
            definitions
        }
    }

    #[test]
    fn storefront_contract_round_trips_json() {
        let contract = storefront_contract();
        let actual = serde_json::to_value(&contract).unwrap();

        assert_eq!(
            actual,
            json!({
                "format": "seam.contract@1",
                "name": "storefront",
                "version": 1,
                "boundaries": [
                    {
                        "function": "fetch",
                        "request": "StorefrontRequest",
                        "response": "StorefrontResponse"
                    }
                ],
                "definitions": [
                    {
                        "kind": "record",
                        "name": "StorefrontRequest",
                        "fields": {
                            "body": {
                                "kind": "option",
                                "item": { "kind": "primitive", "name": "String" }
                            },
                            "headers": {
                                "kind": "map",
                                "key": { "kind": "primitive", "name": "String" },
                                "value": { "kind": "primitive", "name": "String" }
                            },
                            "method": { "kind": "primitive", "name": "String" },
                            "path": { "kind": "primitive", "name": "String" },
                            "pathname": { "kind": "primitive", "name": "String" },
                            "url": { "kind": "primitive", "name": "String" }
                        }
                    },
                    {
                        "kind": "record",
                        "name": "StorefrontResponse",
                        "fields": {
                            "body": { "kind": "primitive", "name": "String" },
                            "body_base64": {
                                "kind": "option",
                                "item": { "kind": "primitive", "name": "String" }
                            },
                            "headers": {
                                "kind": "map",
                                "key": { "kind": "primitive", "name": "String" },
                                "value": { "kind": "primitive", "name": "String" }
                            },
                            "status": { "kind": "primitive", "name": "Int" },
                            "upgrade": {
                                "kind": "option",
                                "item": {
                                    "kind": "map",
                                    "key": { "kind": "primitive", "name": "String" },
                                    "value": { "kind": "primitive", "name": "String" }
                                }
                            }
                        }
                    }
                ]
            })
        );

        let json = serde_json::to_string_pretty(&contract).unwrap();
        let decoded: SeamContract = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, contract);
    }

    #[test]
    fn rust_enum_maps_to_seam_enum_with_variant_payloads() {
        let actual = serde_json::to_value(FulfillmentKind::seam_definitions()).unwrap();

        assert_eq!(
            actual,
            json!([
                {
                    "kind": "enum",
                    "name": "FulfillmentKind",
                    "variants": [
                        {
                            "name": "Digital"
                        },
                        {
                            "name": "Physical",
                            "fields": {
                                "address": {
                                    "kind": "named",
                                    "name": "ShippingAddress"
                                },
                                "carrier": {
                                    "kind": "primitive",
                                    "name": "String"
                                }
                            }
                        },
                        {
                            "name": "Backorder",
                            "fields": {
                                "restock_days": {
                                    "kind": "primitive",
                                    "name": "Int"
                                }
                            }
                        }
                    ]
                },
                {
                    "kind": "record",
                    "name": "ShippingAddress",
                    "fields": {
                        "city": {
                            "kind": "primitive",
                            "name": "String"
                        },
                        "street": {
                            "kind": "primitive",
                            "name": "String"
                        }
                    }
                }
            ])
        );
    }

    #[test]
    fn nested_struct_field_maps_to_named_record_definition() {
        let actual = serde_json::to_value(OrderSummary::seam_definitions()).unwrap();

        assert_eq!(
            actual,
            json!([
                {
                    "kind": "record",
                    "name": "OrderSummary",
                    "fields": {
                        "id": {
                            "kind": "primitive",
                            "name": "String"
                        },
                        "shipping": {
                            "kind": "named",
                            "name": "ShippingAddress"
                        }
                    }
                },
                {
                    "kind": "record",
                    "name": "ShippingAddress",
                    "fields": {
                        "city": {
                            "kind": "primitive",
                            "name": "String"
                        },
                        "street": {
                            "kind": "primitive",
                            "name": "String"
                        }
                    }
                }
            ])
        );
    }
}
