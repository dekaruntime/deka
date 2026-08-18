use super::*;

impl<'a> JsSubsetEmitter<'a> {
    pub(super) fn emit_struct_schema(&self, members: &[ClassMember<'_>]) -> String {
        let mut fields = Vec::new();
        for member in members {
            match member {
                ClassMember::Property { ty, entries, .. } => {
                    for entry in *entries {
                        let (schema, optional) = match ty {
                            Some(ty) => self.emit_type_schema(ty),
                            None => ("{ kind: 'unknown' }".to_string(), false),
                        };
                        let name = self.token_name(entry.name);
                        fields.push(format!(
                            "{}: {{ schema: {}, optional: {} }}",
                            json_string(&name),
                            schema,
                            if optional { "true" } else { "false" }
                        ));
                    }
                }
                // RFD 19: embedded structs are stored as a field keyed by the
                // embedded type's name (e.g. `Employee { Person: Person {...} }`).
                ClassMember::Embed { types, .. } => {
                    for ty_name in *types {
                        let name = self.name_last_segment(*ty_name);
                        fields.push(format!(
                            "{}: {{ schema: {{ kind: 'object' }}, optional: false }}",
                            json_string(&name)
                        ));
                    }
                }
                _ => {}
            }
        }
        format!("{{ kind: 'object', fields: {{ {} }} }}", fields.join(", "))
    }

    pub(super) fn emit_struct_methods(
        &mut self,
        members: &[ClassMember<'_>],
    ) -> Result<Vec<(String, String)>, String> {
        let mut methods = Vec::new();
        for member in members {
            if let ClassMember::Method {
                name, params, body, ..
            } = member
            {
                let method_name = self.token_name(name);
                let js_params = params
                    .iter()
                    // RFD 19: `self` is not a real JS parameter -- it binds
                    // to `this` (see the Expr::Variable passthrough), so it
                    // must not appear in the emitted signature at all, or
                    // `b.greet()` (zero call-site args) would leave it
                    // permanently undefined if ever referenced by its own
                    // parameter binding rather than through the this-mapping.
                    .filter(|p| self.token_name(p.name) != "self")
                    .map(|p| self.token_name(p.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let block = self.emit_method_block(params, body)?;
                methods.push((
                    method_name,
                    format!("function({}) {{\n{} }}", js_params, block),
                ));
            }
        }
        Ok(methods)
    }

    fn emit_ds_method_registration(
        &mut self,
        struct_name: &str,
        method_kind: &str,
        methods: &[(String, String)],
    ) -> Result<(), String> {
        if methods.is_empty() {
            return Ok(());
        }
        let mut entries = Vec::new();
        for (name, body) in methods {
            entries.push(format!("{}: {}", json_string(name), body));
        }
        self.body.push_str(&format!(
            "{}.{}({{ {} }});\n",
            struct_name,
            method_kind,
            entries.join(", ")
        ));
        Ok(())
    }

    pub(super) fn emit_ds_impl_calls(
        &mut self,
        struct_name: &str,
        methods: &[(String, String, bool)],
    ) -> Result<(), String> {
        let immutable: Vec<(String, String)> = methods
            .iter()
            .filter(|(_, _, is_mut)| !is_mut)
            .map(|(n, b, _)| (n.clone(), b.clone()))
            .collect();
        let mutable: Vec<(String, String)> = methods
            .iter()
            .filter(|(_, _, is_mut)| *is_mut)
            .map(|(n, b, _)| (n.clone(), b.clone()))
            .collect();
        self.emit_ds_method_registration(struct_name, "impl", &immutable)?;
        self.emit_ds_method_registration(struct_name, "implMut", &mutable)?;
        Ok(())
    }

    pub(super) fn emit_type_schema(&self, ty: &AstType<'_>) -> (String, bool) {
        match ty {
            AstType::Simple(tok) => match self.token_name(tok).as_str() {
                "string" => ("{ kind: 'string' }".to_string(), false),
                "bytes" => ("{ kind: 'object' }".to_string(), false),
                "int" | "float" | "number" => ("{ kind: 'number' }".to_string(), false),
                "bool" | "boolean" => ("{ kind: 'boolean' }".to_string(), false),
                _ => ("{ kind: 'unknown' }".to_string(), false),
            },
            AstType::Name(_) => ("{ kind: 'object' }".to_string(), false),
            AstType::Nullable(inner) => {
                let (inner_schema, _) = self.emit_type_schema(inner);
                (
                    format!("{{ kind: 'optional', inner: {} }}", inner_schema),
                    true,
                )
            }
            AstType::Union(parts) => {
                let schemas = parts
                    .iter()
                    .map(|part| self.emit_type_schema(part).0)
                    .collect::<Vec<_>>();
                (
                    format!("{{ kind: 'union', anyOf: [{}] }}", schemas.join(", ")),
                    false,
                )
            }
            AstType::Intersection(_parts) => ("{ kind: 'object' }".to_string(), false),
            AstType::ObjectShape(shape_fields) => {
                let fields = shape_fields
                    .iter()
                    .map(|field| {
                        let (inner, opt) = self.emit_type_schema(field.ty);
                        format!(
                            "{}: {{ schema: {}, optional: {} }}",
                            json_string(&self.token_name(field.name)),
                            inner,
                            if field.optional || opt {
                                "true"
                            } else {
                                "false"
                            }
                        )
                    })
                    .collect::<Vec<_>>();
                (
                    format!("{{ kind: 'object', fields: {{ {} }} }}", fields.join(", ")),
                    false,
                )
            }
            AstType::Applied { base, args } => {
                if let AstType::Simple(tok) = *base {
                    let base_name = self.token_name(tok);
                    if base_name == "Option" {
                        let inner = args
                            .first()
                            .map(|t| self.emit_type_schema(t).0)
                            .unwrap_or_else(|| "{ kind: 'unknown' }".to_string());
                        return (format!("{{ kind: 'optional', inner: {} }}", inner), true);
                    }
                    if base_name == "array" || base_name == "Vec" {
                        let inner = args
                            .first()
                            .map(|t| self.emit_type_schema(t).0)
                            .unwrap_or_else(|| "{ kind: 'unknown' }".to_string());
                        return (format!("{{ kind: 'array', item: {} }}", inner), false);
                    }
                }
                ("{ kind: 'unknown' }".to_string(), false)
            }
        }
    }
}
