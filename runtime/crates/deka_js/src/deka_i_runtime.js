const __deka_i = (() => {
  const ok = (data) => ({ success: true, data })
  const fail = (message) => ({ success: false, error: { issues: [{ message }] } })

  const parseSchema = (schema, value) => {
    if (!schema || typeof schema !== 'object') return ok(value)
    switch (schema.kind) {
      case 'string':
        return typeof value === 'string' ? ok(value) : fail('expected string')
      case 'number':
        return typeof value === 'number' ? ok(value) : fail('expected number')
      case 'boolean':
        return typeof value === 'boolean' ? ok(value) : fail('expected boolean')
      case 'optional':
        return value == null ? ok(value) : parseSchema(schema.inner, value)
      case 'array': {
        if (!Array.isArray(value)) return fail('expected array')
        for (let idx = 0; idx < value.length; idx += 1) {
          const item = parseSchema(schema.item || { kind: 'unknown' }, value[idx])
          if (!item.success) return fail('invalid array item at ' + idx)
        }
        return ok(value)
      }
      case 'object': {
        if (!value || typeof value !== 'object' || Array.isArray(value)) return fail('expected object')
        const fields = schema.fields || {}
        for (const [name, field] of Object.entries(fields)) {
          const has = Object.prototype.hasOwnProperty.call(value, name)
          if (!has) {
            if (field && field.optional) continue
            return fail('missing field ' + name)
          }
          const checked = parseSchema((field && field.schema) || { kind: 'unknown' }, value[name])
          if (!checked.success) return fail('invalid field ' + name)
        }
        return ok(value)
      }
      case 'union': {
        for (const item of schema.anyOf || []) {
          const checked = parseSchema(item, value)
          if (checked.success) return checked
        }
        return fail('no union branch matched')
      }
      default:
        return ok(value)
    }
  }

  const wrap = (schema) => ({
    __schema: schema,
    parse(value) {
      const result = parseSchema(schema, value)
      if (!result.success) throw new Error(result.error.issues[0].message)
      return result.data
    },
    safeParse(value) {
      return parseSchema(schema, value)
    },
    optional() {
      return wrap({ kind: 'optional', inner: schema })
    },
    array() {
      return wrap({ kind: 'array', item: schema })
    },
  })

  return {
    string: () => wrap({ kind: 'string' }),
    number: () => wrap({ kind: 'number' }),
    boolean: () => wrap({ kind: 'boolean' }),
    array: (schema) =>
      wrap({ kind: 'array', item: schema && schema.__schema ? schema.__schema : schema || { kind: 'unknown' } }),
    object: (shape) => {
      const fields = {}
      for (const [name, schema] of Object.entries(shape || {})) {
        fields[name] = { schema: schema && schema.__schema ? schema.__schema : schema || { kind: 'unknown' }, optional: false }
      }
      return wrap({ kind: 'object', fields })
    },
    optional: (schema) => wrap({ kind: 'optional', inner: schema && schema.__schema ? schema.__schema : schema }),
    get: (name) => {
      const schema = (__phpxTypeRegistry || {})[name]
      if (!schema) return null
      return wrap(schema)
    },
    parse: (name, value) => {
      const schema = (__phpxTypeRegistry || {})[name]
      if (!schema) throw new Error('unknown schema ' + name)
      return wrap(schema).parse(value)
    },
    safeParse: (name, value) => {
      const schema = (__phpxTypeRegistry || {})[name]
      if (!schema) return fail('unknown schema ' + name)
      return wrap(schema).safeParse(value)
    },
    registry: () => (__phpxTypeRegistry || {}),
  }
})()
