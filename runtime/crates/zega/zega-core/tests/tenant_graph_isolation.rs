use std::collections::HashMap;
use zega_core::{ExprValue, PolicyCondition, PolicyExpr, PolicyTargets, Value, Zega, ZegaContext};

fn claims(shop_id: &str) -> HashMap<String, Value> {
    HashMap::from([("shop_id".to_string(), Value::String(shop_id.to_string()))])
}

fn field<'a>(row: &'a zega_core::Row, binding: &str, prop: &str) -> Option<&'a Value> {
    match row.fields.get(binding) {
        Some(Value::Map(map)) => map.get(prop),
        _ => None,
    }
}

fn commerce_zega() -> Zega {
    Zega::in_memory()
        .policy(
            "shop_tenant",
            PolicyTargets::Labels(vec!["Shop".to_string()]),
            PolicyCondition::AllowWhen(PolicyExpr::Eq(
                ExprValue::ContextField(".shop_id".to_string()),
                ExprValue::NodeField("node.shop_id".to_string()),
            )),
        )
        .policy(
            "order_tenant",
            PolicyTargets::Labels(vec!["Order".to_string()]),
            PolicyCondition::AllowWhen(PolicyExpr::Eq(
                ExprValue::ContextField(".shop_id".to_string()),
                ExprValue::NodeField("node.shop_id".to_string()),
            )),
        )
        .build()
        .unwrap()
}

fn seed_two_shops(zega: &Zega) {
    zega.query_with_context(
        "CREATE (s:Shop {id: 'shop_alpha', shop_id: 'shop_alpha'})",
        HashMap::new(),
        ZegaContext::system(),
    )
    .unwrap();
    zega.query_with_context(
        "CREATE (s:Shop {id: 'shop_beta', shop_id: 'shop_beta'})",
        HashMap::new(),
        ZegaContext::system(),
    )
    .unwrap();
    zega.query_with_context(
        "MATCH (s:Shop {id: 'shop_alpha'}) CREATE (s)<-[:FROM]-(o:Order {id: 'order_alpha', shop_id: 'shop_alpha'})",
        HashMap::new(),
        ZegaContext::system(),
    )
    .unwrap();
    zega.query_with_context(
        "MATCH (s:Shop {id: 'shop_beta'}) CREATE (s)<-[:FROM]-(o:Order {id: 'order_beta', shop_id: 'shop_beta'})",
        HashMap::new(),
        ZegaContext::system(),
    )
    .unwrap();
}

#[test]
fn tenant_order_match_cannot_return_another_shops_order() {
    let zega = commerce_zega();
    seed_two_shops(&zega);

    let rows = zega
        .query_with_context(
            "MATCH (o:Order) RETURN o",
            HashMap::new(),
            ZegaContext::claims(claims("shop_alpha")),
        )
        .unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(
        field(&rows[0], "o", "id"),
        Some(&Value::String("order_alpha".to_string()))
    );
}

#[test]
fn tenant_shop_traversal_cannot_cross_to_another_shops_order() {
    let zega = commerce_zega();
    seed_two_shops(&zega);

    let rows = zega
        .query_with_context(
            "MATCH (s:Shop)<-[:FROM]-(o:Order) RETURN s, o",
            HashMap::new(),
            ZegaContext::claims(claims("shop_beta")),
        )
        .unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(
        field(&rows[0], "s", "id"),
        Some(&Value::String("shop_beta".to_string()))
    );
    assert_eq!(
        field(&rows[0], "o", "id"),
        Some(&Value::String("order_beta".to_string()))
    );
}
