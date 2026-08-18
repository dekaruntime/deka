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

    /// Collect DekaScript receiver methods in a pre-pass so they can be emitted
    /// after every struct factory has been declared.
    pub(super) fn collect_ds_receiver_methods(
        &mut self,
        program: &Program<'_>,
    ) -> Result<(), String> {
        for stmt in program.statements {
            let Stmt::ReceiverMethod {
                name,
                is_async,
                receiver,
                params,
                body,
                ..
            } = stmt
            else {
                continue;
            };
            let method_name = self.token_name(name);
            let receiver_name = self.token_name(receiver.var);
            let receiver_type_name = match receiver.ty {
                AstType::Name(name) => self.name_last_segment(*name),
                AstType::Simple(tok) => self.token_name(tok),
                _ => {
                    return Err(
                        "receiver type must be a struct name in JS subset emitter".to_string(),
                    )
                }
            };
            let js_params = std::iter::once(receiver_name)
                .chain(params.iter().map(|p| self.token_name(p.name)))
                .collect::<Vec<_>>()
                .join(", ");
            let block = self.emit_receiver_method_block(receiver, params, body)?;
            let async_kw = if *is_async { "async " } else { "" };
            self.uses_deka_struct_helpers = true;
            let func_expr = format!(
                "{}function {}({}) {{\n{} }}",
                async_kw, method_name, js_params, block
            );
            self.ds_struct_methods
                .entry(receiver_type_name)
                .or_default()
                .push(DsMethod {
                    name: method_name,
                    body: func_expr,
                    is_mut: receiver.is_mut,
                });
        }
        Ok(())
    }

    /// Emit all DekaScript struct method registrations (own, receiver, and
    /// promoted embedded methods) after every struct factory is declared.
    pub(super) fn emit_ds_method_registrations(&mut self) -> Result<(), String> {
        let struct_names: Vec<String> = self
            .ds_struct_methods
            .keys()
            .chain(self.struct_embeds.keys())
            .cloned()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        for struct_name in struct_names {
            let mut methods: Vec<DsMethod> = self
                .ds_struct_methods
                .get(&struct_name)
                .cloned()
                .unwrap_or_default();
            let mut seen: HashSet<String> = methods.iter().map(|m| m.name.clone()).collect();
            let mut visited = HashSet::new();
            if let Some(embeds) = self.struct_embeds.get(&struct_name).cloned() {
                for embed in embeds {
                    let mut path = Vec::new();
                    self.collect_promoted_ds_methods(
                        &embed,
                        &mut path,
                        &mut methods,
                        &mut seen,
                        &mut visited,
                    )?;
                }
            }
            if !methods.is_empty() {
                self.uses_deka_struct_helpers = true;
            }
            let immutable: Vec<(String, String)> = methods
                .iter()
                .filter(|m| !m.is_mut)
                .map(|m| (m.name.clone(), m.body.clone()))
                .collect();
            let mutable: Vec<(String, String)> = methods
                .iter()
                .filter(|m| m.is_mut)
                .map(|m| (m.name.clone(), m.body.clone()))
                .collect();
            self.emit_ds_method_registration(&struct_name, "impl", &immutable)?;
            self.emit_ds_method_registration(&struct_name, "implMut", &mutable)?;
        }
        Ok(())
    }

    fn collect_promoted_ds_methods(
        &self,
        embed: &str,
        path: &mut Vec<String>,
        out: &mut Vec<DsMethod>,
        seen: &mut HashSet<String>,
        visited: &mut HashSet<String>,
    ) -> Result<(), String> {
        if !visited.insert(embed.to_string()) {
            return Ok(());
        }
        path.push(embed.to_string());
        if let Some(methods) = self.ds_struct_methods.get(embed) {
            let access = path.join(".");
            for method in methods {
                if !seen.insert(method.name.clone()) {
                    continue;
                }
                let wrapper = format!(
                    "function(...args) {{ return this.{}.{}{}; }}",
                    access, method.name, "(...args)"
                );
                out.push(DsMethod {
                    name: method.name.clone(),
                    body: wrapper,
                    is_mut: method.is_mut,
                });
            }
        }
        if let Some(embeds) = self.struct_embeds.get(embed) {
            for next in embeds.clone() {
                self.collect_promoted_ds_methods(&next, path, out, seen, visited)?;
            }
        }
        path.pop();
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
