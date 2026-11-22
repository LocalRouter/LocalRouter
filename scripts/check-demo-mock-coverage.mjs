#!/usr/bin/env node
/**
 * Compares three sets of Tauri command names:
 *
 *   registered - the `tauri::generate_handler![...]` list in src-tauri/src/main.rs
 *   invoked    - commands the frontend calls through `invoke(...)` in src/ and website/src/
 *   mocked     - keys of the website demo dispatcher (`mockHandlers` in
 *                website/src/components/demo/TauriMockSetup.ts, spreads included)
 *
 * and prints:
 *
 *   1. commands invoked by the frontend but unmocked (the demo shows a
 *      "not implemented" toast for these)
 *   2. commands registered but never invoked by the frontend
 *   3. commands mocked but not registered (stale mocks)
 *
 * Frontend sources are read with the TypeScript parser, so comments, strings
 * that merely mention a command, generics (`invoke<T>('name')`), multi-line
 * calls and aliased imports are handled. A command name that is not a literal
 * at the call site is resolved through constants, conditionals, wrapper
 * functions and typed unions where possible; whatever cannot be resolved is
 * reported as a dynamic call site instead of being guessed.
 *
 * Usage:
 *   node scripts/check-demo-mock-coverage.mjs           # report; exit 1 on any gap the unit test rejects
 *   node scripts/check-demo-mock-coverage.mjs --json    # machine-readable
 *   node scripts/check-demo-mock-coverage.mjs --where   # include call sites
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const HANDLER_FILE = 'src-tauri/src/main.rs'
const MOCK_ENTRY = 'website/src/components/demo/TauriMockSetup.ts'
const MOCK_OBJECT = 'mockHandlers'
const FRONTEND_ROOTS = ['src', 'website/src']
const INVOKE_MODULE = '@tauri-apps/api/core'

/**
 * Commands the frontend invokes that are deliberately left without a demo
 * mock. Every entry needs a reason. Keep this empty unless a command truly
 * cannot be represented in the browser demo.
 */
export const DELIBERATELY_UNMOCKED = {
  // 'command_name': 'why the demo cannot or must not answer it',
}

// ---------------------------------------------------------------------------
// Registered commands (Rust)
// ---------------------------------------------------------------------------

/** Removes `//` and `/* *\/` comments from Rust source (string-literal aware). */
function stripRustComments(source) {
  let out = ''
  let i = 0
  while (i < source.length) {
    const two = source.slice(i, i + 2)
    if (two === '//') {
      while (i < source.length && source[i] !== '\n') i++
    } else if (two === '/*') {
      let depth = 1
      i += 2
      while (i < source.length && depth > 0) {
        if (source.slice(i, i + 2) === '/*') { depth++; i += 2 }
        else if (source.slice(i, i + 2) === '*/') { depth--; i += 2 }
        else i++
      }
    } else if (source[i] === '"') {
      const start = i
      i++
      while (i < source.length && source[i] !== '"') i += source[i] === '\\' ? 2 : 1
      i++
      out += source.slice(start, i)
    } else {
      out += source[i++]
    }
  }
  return out
}

export function registeredCommands(root = REPO_ROOT) {
  const source = stripRustComments(fs.readFileSync(path.join(root, HANDLER_FILE), 'utf8'))
  const names = new Set()
  const marker = 'generate_handler!['
  let from = 0
  let found = false
  for (;;) {
    const start = source.indexOf(marker, from)
    if (start === -1) break
    found = true
    let depth = 1
    let i = start + marker.length
    while (i < source.length && depth > 0) {
      if (source[i] === '[') depth++
      else if (source[i] === ']') depth--
      i++
    }
    const body = source.slice(start + marker.length, i - 1)
    for (const raw of body.split(',')) {
      // Drop attributes such as `#[cfg(...)]` in front of an entry.
      const entry = raw.replace(/#\[[^\]]*\]/g, '').trim()
      if (!entry) continue
      const name = entry.split('::').pop().trim()
      if (!/^[a-z_][a-z0-9_]*$/.test(name)) {
        throw new Error(`Unrecognized generate_handler! entry: "${entry}"`)
      }
      names.add(name)
    }
    from = i
  }
  if (!found) throw new Error(`No generate_handler! list found in ${HANDLER_FILE}`)
  return names
}

// ---------------------------------------------------------------------------
// TypeScript helpers
// ---------------------------------------------------------------------------

const sourceCache = new Map()

function parse(file) {
  let sf = sourceCache.get(file)
  if (!sf) {
    const kind = file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS
    sf = ts.createSourceFile(file, fs.readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true, kind)
    sourceCache.set(file, sf)
  }
  return sf
}

function unwrap(node) {
  let n = node
  while (
    n && (ts.isParenthesizedExpression(n) || ts.isAsExpression(n) || ts.isSatisfiesExpression(n) ||
      ts.isNonNullExpression(n) || ts.isTypeAssertionExpression(n))
  ) n = n.expression
  return n
}

function resolveModule(fromFile, specifier, root) {
  let base
  if (specifier.startsWith('.')) base = path.resolve(path.dirname(fromFile), specifier)
  else if (specifier.startsWith('@app/')) base = path.join(root, 'src', specifier.slice('@app/'.length))
  else if (specifier.startsWith('@/')) {
    // `@/` is the root app's `src/`; inside the website it is `website/src/`.
    const inWebsite = path.relative(path.join(root, 'website'), fromFile).startsWith('src' + path.sep)
    base = path.join(root, inWebsite ? 'website/src' : 'src', specifier.slice(2))
  } else return null
  for (const candidate of [base, `${base}.ts`, `${base}.tsx`, path.join(base, 'index.ts'), path.join(base, 'index.tsx')]) {
    if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) return candidate
  }
  return null
}

/** Top-level `const name = <initializer>` / `function name` declarations of a file. */
function topLevelDeclarations(sf) {
  const decls = new Map()
  for (const stmt of sf.statements) {
    if (ts.isVariableStatement(stmt)) {
      for (const d of stmt.declarationList.declarations) {
        if (ts.isIdentifier(d.name)) decls.set(d.name.text, d)
      }
    } else if (ts.isFunctionDeclaration(stmt) && stmt.name) {
      decls.set(stmt.name.text, stmt)
    }
  }
  return decls
}

/** Local name -> { file, name } for the named imports of a file. */
function namedImports(sf, root) {
  const imports = new Map()
  for (const stmt of sf.statements) {
    if (!ts.isImportDeclaration(stmt) || !stmt.importClause?.namedBindings) continue
    if (!ts.isNamedImports(stmt.importClause.namedBindings)) continue
    const specifier = stmt.moduleSpecifier.text
    for (const el of stmt.importClause.namedBindings.elements) {
      imports.set(el.name.text, { specifier, name: (el.propertyName ?? el.name).text, file: resolveModule(sf.fileName, specifier, root) })
    }
  }
  return imports
}

// ---------------------------------------------------------------------------
// Mocked commands (demo dispatcher)
// ---------------------------------------------------------------------------

function objectLiteralKeys(objectLiteral, sf, root, keys, seen) {
  for (const prop of objectLiteral.properties) {
    if (ts.isSpreadAssignment(prop)) {
      const target = unwrap(prop.expression)
      if (!ts.isIdentifier(target)) {
        throw new Error(`${sf.fileName}: cannot follow spread "${prop.getText(sf)}" in the mock handler object`)
      }
      exportedObjectKeys(sf, target.text, root, keys, seen)
    } else if (prop.name && (ts.isStringLiteralLike(prop.name) || ts.isIdentifier(prop.name))) {
      keys.add(prop.name.text)
    } else {
      throw new Error(`${sf.fileName}: unsupported key "${prop.name?.getText(sf)}" in the mock handler object`)
    }
  }
}

function exportedObjectKeys(sf, name, root, keys, seen) {
  const id = `${sf.fileName}#${name}`
  if (seen.has(id)) return
  seen.add(id)
  const decl = topLevelDeclarations(sf).get(name)
  if (decl && ts.isVariableDeclaration(decl) && decl.initializer) {
    const init = unwrap(decl.initializer)
    if (!ts.isObjectLiteralExpression(init)) {
      throw new Error(`${sf.fileName}: "${name}" is not an object literal`)
    }
    objectLiteralKeys(init, sf, root, keys, seen)
    return
  }
  const imported = namedImports(sf, root).get(name)
  if (imported?.file) {
    exportedObjectKeys(parse(imported.file), imported.name, root, keys, seen)
    return
  }
  throw new Error(`${sf.fileName}: cannot find the object "${name}"`)
}

/** Every key the demo dispatcher answers, `plugin:*` keys excluded. */
export function mockedCommands(root = REPO_ROOT) {
  const sf = parse(path.join(root, MOCK_ENTRY))
  const keys = new Set()
  exportedObjectKeys(sf, MOCK_OBJECT, root, keys, new Set())
  // Handlers added after the literal: `mockHandlers['name'] = ...` / `mockHandlers.name = ...`.
  const visit = (node) => {
    if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken) {
      const left = node.left
      if (ts.isElementAccessExpression(left) && ts.isIdentifier(left.expression) && left.expression.text === MOCK_OBJECT &&
        ts.isStringLiteralLike(left.argumentExpression)) keys.add(left.argumentExpression.text)
      if (ts.isPropertyAccessExpression(left) && ts.isIdentifier(left.expression) && left.expression.text === MOCK_OBJECT) keys.add(left.name.text)
    }
    ts.forEachChild(node, visit)
  }
  visit(sf)
  return new Set([...keys].filter(k => !k.startsWith('plugin:')))
}

// ---------------------------------------------------------------------------
// Invoked commands (frontend)
// ---------------------------------------------------------------------------

function listSourceFiles(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === 'node_modules' || entry.name === 'dist' || entry.name.startsWith('.')) continue
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) listSourceFiles(full, out)
    else if (/\.(ts|tsx)$/.test(entry.name) && !entry.name.endsWith('.d.ts')) out.push(full)
  }
  return out
}

/** String literals a type node can stand for (`'a' | 'b'`, aliases of those), or null. */
function typeLiterals(typeNode, sf, root, depth = 0) {
  if (!typeNode || depth > 6) return null
  if (ts.isParenthesizedTypeNode(typeNode)) return typeLiterals(typeNode.type, sf, root, depth + 1)
  if (ts.isLiteralTypeNode(typeNode) && ts.isStringLiteralLike(typeNode.literal)) return [typeNode.literal.text]
  if (ts.isUnionTypeNode(typeNode)) {
    const all = []
    for (const member of typeNode.types) {
      const part = typeLiterals(member, sf, root, depth + 1)
      if (!part) return null
      all.push(...part)
    }
    return all
  }
  if (ts.isTypeReferenceNode(typeNode) && ts.isIdentifier(typeNode.typeName)) {
    const name = typeNode.typeName.text
    for (const stmt of sf.statements) {
      if (ts.isTypeAliasDeclaration(stmt) && stmt.name.text === name) return typeLiterals(stmt.type, sf, root, depth + 1)
    }
    const imported = namedImports(sf, root).get(name)
    if (imported?.file) {
      const other = parse(imported.file)
      for (const stmt of other.statements) {
        if (ts.isTypeAliasDeclaration(stmt) && stmt.name.text === imported.name) return typeLiterals(stmt.type, other, root, depth + 1)
      }
    }
  }
  return null
}

/** The declaration an identifier refers to, searching enclosing scopes then imports. */
function findDeclaration(identifier, sf, root) {
  const name = identifier.text
  for (let scope = identifier.parent; scope; scope = scope.parent) {
    if (ts.isFunctionLike(scope)) {
      const index = scope.parameters.findIndex(p => ts.isIdentifier(p.name) && p.name.text === name)
      if (index !== -1) return { kind: 'param', fn: scope, index, param: scope.parameters[index], sf }
      // Destructured props: `function C({ command }: Props)`.
      for (const p of scope.parameters) {
        if (ts.isObjectBindingPattern(p.name)) {
          for (const el of p.name.elements) {
            if (ts.isIdentifier(el.name) && el.name.text === name) {
              return { kind: 'prop', fn: scope, index: scope.parameters.indexOf(p), prop: (el.propertyName ?? el.name).getText(sf), sf }
            }
          }
        }
      }
    }
    if (ts.isBlock(scope) || ts.isSourceFile(scope) || ts.isCaseClause(scope) || ts.isDefaultClause(scope) || ts.isModuleBlock(scope)) {
      for (const stmt of scope.statements) {
        if (ts.isVariableStatement(stmt)) {
          for (const d of stmt.declarationList.declarations) {
            if (ts.isIdentifier(d.name) && d.name.text === name) {
              return { kind: 'var', decl: d, isConst: (stmt.declarationList.flags & ts.NodeFlags.Const) !== 0, scope, sf }
            }
            // `const { command } = source`
            if (ts.isObjectBindingPattern(d.name) && d.initializer) {
              for (const el of d.name.elements) {
                if (ts.isIdentifier(el.name) && el.name.text === name) {
                  return { kind: 'destructured', source: d.initializer, prop: (el.propertyName ?? el.name).getText(sf), sf }
                }
              }
            }
          }
        }
      }
    }
    if (ts.isForOfStatement(scope) && ts.isVariableDeclarationList(scope.initializer)) {
      const d = scope.initializer.declarations[0]
      if (ts.isIdentifier(d.name) && d.name.text === name) return { kind: 'forOf', iterable: scope.expression, sf }
    }
  }
  const imported = namedImports(sf, root).get(name)
  if (imported?.file) {
    const other = parse(imported.file)
    const decl = topLevelDeclarations(other).get(imported.name)
    if (decl && ts.isVariableDeclaration(decl)) return { kind: 'var', decl, isConst: true, scope: other, sf: other }
  }
  return null
}

/**
 * Resolves an expression to the set of command names it can evaluate to.
 * Returns null when that cannot be established statically. Wrapper parameters
 * are returned as `{ wrapper }` markers for the caller to chase.
 */
function resolveNames(expr, sf, root, ctx, depth = 0) {
  if (depth > 8) return null
  const node = unwrap(expr)
  if (!node) return null
  if (ts.isStringLiteralLike(node)) return [node.text]
  if (ts.isConditionalExpression(node)) {
    const a = resolveNames(node.whenTrue, sf, root, ctx, depth + 1)
    const b = resolveNames(node.whenFalse, sf, root, ctx, depth + 1)
    return a && b ? [...a, ...b] : null
  }
  if (ts.isBinaryExpression(node) && [ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken].includes(node.operatorToken.kind)) {
    const a = resolveNames(node.left, sf, root, ctx, depth + 1)
    const b = resolveNames(node.right, sf, root, ctx, depth + 1)
    return a && b ? [...a, ...b] : null
  }
  if (ts.isArrayLiteralExpression(node)) {
    const all = []
    for (const el of node.elements) {
      const part = resolveNames(el, sf, root, ctx, depth + 1)
      if (!part) return null
      all.push(...part)
    }
    return all
  }
  // `TABLE[key]` / `TABLE.key` where TABLE is a const object of command names:
  // any of its values can be selected.
  if (ts.isElementAccessExpression(node) || ts.isPropertyAccessExpression(node)) {
    const target = unwrap(node.expression)
    if (ts.isIdentifier(target)) {
      const found = findDeclaration(target, sf, root)
      const init = found?.kind === 'var' && found.isConst && found.decl.initializer ? unwrap(found.decl.initializer) : null
      if (init && ts.isObjectLiteralExpression(init)) {
        if (ts.isPropertyAccessExpression(node)) {
          const prop = init.properties.find(p => ts.isPropertyAssignment(p) && p.name.getText(found.sf).replace(/['"]/g, '') === node.name.text)
          return prop ? resolveNames(prop.initializer, found.sf, root, ctx, depth + 1) : null
        }
        const all = []
        for (const p of init.properties) {
          if (!ts.isPropertyAssignment(p)) return null
          const part = resolveNames(p.initializer, found.sf, root, ctx, depth + 1)
          if (!part) return null
          all.push(...part)
        }
        return all
      }
    }
    return null
  }
  if (ts.isIdentifier(node)) {
    const found = findDeclaration(node, sf, root)
    if (!found) return null
    if (found.kind === 'var') {
      const fromType = typeLiterals(found.decl.type, found.sf, root)
      if (fromType) return fromType
      if (!found.decl.initializer) return null
      const initial = resolveNames(found.decl.initializer, found.sf, root, ctx, depth + 1)
      if (!initial || found.isConst) return initial
      // `let cmd = 'a'; if (...) cmd = 'b'` - add every assignment in scope.
      const all = [...initial]
      let ok = true
      const visit = (n) => {
        if (ts.isBinaryExpression(n) && n.operatorToken.kind === ts.SyntaxKind.EqualsToken && ts.isIdentifier(n.left) && n.left.text === node.text) {
          const part = resolveNames(n.right, found.sf, root, ctx, depth + 1)
          if (part) all.push(...part)
          else ok = false
        }
        ts.forEachChild(n, visit)
      }
      visit(found.scope)
      return ok ? all : null
    }
    if (found.kind === 'forOf') return resolveNames(found.iterable, found.sf, root, ctx, depth + 1)
    if (found.kind === 'prop') return wrapperMarker(found.fn, found.index, found.prop, found.sf, ctx)
    if (found.kind === 'destructured') {
      const source = unwrap(found.source)
      // `const { command } = options` where `options` is a parameter.
      if (ts.isIdentifier(source)) {
        const origin = findDeclaration(source, found.sf, root)
        if (origin?.kind === 'param') return wrapperMarker(origin.fn, origin.index, found.prop, origin.sf, ctx)
        return null
      }
      // `const { command } = pick(...)` where `pick` returns `{ command, ... }`.
      if (ts.isCallExpression(source) && ts.isIdentifier(unwrap(source.expression))) {
        const fn = functionOf(findDeclaration(unwrap(source.expression), found.sf, root))
        if (!fn) return null
        const all = []
        for (const returned of returnedExpressions(fn)) {
          const value = unwrap(returned)
          if (!value || !ts.isObjectLiteralExpression(value)) return null
          const prop = value.properties.find(p => p.name && p.name.getText(found.sf) === found.prop)
          if (!prop) return null
          const part = ts.isShorthandPropertyAssignment(prop)
            ? resolveNames(prop.name, found.sf, root, ctx, depth + 1)
            : ts.isPropertyAssignment(prop) ? resolveNames(prop.initializer, found.sf, root, ctx, depth + 1) : null
          if (!part) return null
          all.push(...part)
        }
        return all
      }
      return null
    }
    if (found.kind === 'param') {
      const fromType = typeLiterals(found.param.type, found.sf, root)
      if (fromType) return fromType
      return wrapperMarker(found.fn, found.index, null, found.sf, ctx)
    }
  }
  return null
}

/**
 * Marks `fn` as a wrapper whose argument `index` (or that argument's `prop`
 * when it is an options object) carries the command name. Its call sites are
 * resolved afterwards.
 */
function wrapperMarker(fn, index, prop, sf, ctx) {
  const name = functionName(fn)
  if (!name) return null
  ctx.wrappers.push({ name, index, prop, file: sf.fileName, exported: isExported(fn) })
  return []
}

/** The function a declaration introduces, seeing through `useCallback(fn, deps)`. */
function functionOf(found) {
  if (!found || found.kind !== 'var' || !found.decl.initializer) return null
  let init = unwrap(found.decl.initializer)
  if (ts.isCallExpression(init) && init.arguments.length) init = unwrap(init.arguments[0])
  return ts.isArrowFunction(init) || ts.isFunctionExpression(init) ? init : null
}

/** Expressions a function can return (nested functions excluded). */
function returnedExpressions(fn) {
  if (!ts.isBlock(fn.body)) return [fn.body]
  const out = []
  const visit = (n) => {
    if (ts.isFunctionLike(n)) return
    if (ts.isReturnStatement(n)) out.push(n.expression)
    ts.forEachChild(n, visit)
  }
  ts.forEachChild(fn.body, visit)
  return out
}

function functionName(fn) {
  if (fn.name && ts.isIdentifier(fn.name)) return fn.name.text
  const parent = fn.parent
  if (parent && ts.isVariableDeclaration(parent) && ts.isIdentifier(parent.name)) return parent.name.text
  if (parent && ts.isCallExpression(parent) && parent.parent && ts.isVariableDeclaration(parent.parent) && ts.isIdentifier(parent.parent.name)) {
    return parent.parent.name.text // const f = useCallback((cmd) => invoke(cmd), [])
  }
  return null
}

function isExported(fn) {
  for (let n = fn; n; n = n.parent) {
    if (ts.canHaveModifiers(n) && ts.getModifiers(n)?.some(m => m.kind === ts.SyntaxKind.ExportKeyword)) return true
    if (ts.isSourceFile(n.parent ?? n)) break
  }
  return false
}

/** Local identifiers bound to Tauri's `invoke` in a file. */
function invokeBindings(sf) {
  const names = new Set()
  const namespaces = new Set()
  for (const stmt of sf.statements) {
    if (!ts.isImportDeclaration(stmt) || stmt.moduleSpecifier.text !== INVOKE_MODULE || !stmt.importClause) continue
    if (stmt.importClause.isTypeOnly) continue
    const bindings = stmt.importClause.namedBindings
    if (bindings && ts.isNamedImports(bindings)) {
      for (const el of bindings.elements) {
        if ((el.propertyName ?? el.name).text === 'invoke') names.add(el.name.text)
      }
    } else if (bindings && ts.isNamespaceImport(bindings)) {
      namespaces.add(bindings.name.text)
    }
  }
  return { names, namespaces }
}

export function invokedCommands(root = REPO_ROOT, { exclude = defaultExclude } = {}) {
  const invoked = new Map() // name -> string[] call sites
  const dynamic = [] // call sites whose command name could not be resolved
  const files = FRONTEND_ROOTS.flatMap(dir => listSourceFiles(path.join(root, dir))).filter(f => !exclude(path.relative(root, f)))
  const where = (sf, node) => {
    const { line } = sf.getLineAndCharacterOfPosition(node.getStart(sf))
    return `${path.relative(root, sf.fileName)}:${line + 1}`
  }
  const record = (names, site) => {
    for (const name of names) {
      if (!name) continue // `let command = ""` placeholders
      if (!invoked.has(name)) invoked.set(name, [])
      invoked.get(name).push(site)
    }
  }

  const wrappers = []
  for (const file of files) {
    const sf = parse(file)
    const { names, namespaces } = invokeBindings(sf)
    const dynamicImport = /import\(\s*['"]@tauri-apps\/api\/core['"]\s*\)/.test(sf.text)
    if (names.size === 0 && namespaces.size === 0 && !dynamicImport) continue
    const visit = (node) => {
      if (ts.isCallExpression(node)) {
        const callee = unwrap(node.expression)
        const isInvoke =
          (ts.isIdentifier(callee) && (names.has(callee.text) || (dynamicImport && callee.text === 'invoke'))) ||
          (ts.isPropertyAccessExpression(callee) && callee.name.text === 'invoke' && ts.isIdentifier(callee.expression) &&
            (namespaces.has(callee.expression.text) || dynamicImport))
        if (isInvoke) {
          const site = where(sf, node)
          const ctx = { wrappers: [] }
          const resolved = node.arguments.length ? resolveNames(node.arguments[0], sf, root, ctx) : null
          if (resolved === null) dynamic.push({ site, text: node.arguments[0]?.getText(sf) ?? '' })
          else record(resolved, site)
          // Remember where the name is consumed, to report a wrapper nobody calls.
          wrappers.push(...ctx.wrappers.map(w => ({ ...w, origin: { site, text: node.arguments[0].getText(sf) } })))
        }
      }
      ts.forEachChild(node, visit)
    }
    visit(sf)
  }

  // Chase wrapper functions: `function call(cmd, args) { return invoke(cmd, args) }`.
  const chased = new Set()
  while (wrappers.length) {
    const wrapper = wrappers.pop()
    const key = `${wrapper.file}#${wrapper.name}#${wrapper.index}#${wrapper.prop ?? ''}`
    if (chased.has(key)) continue
    chased.add(key)
    let callSites = 0
    for (const file of files) {
      const sf = parse(file)
      if (file !== wrapper.file) {
        const imported = namedImports(sf, root).get(wrapper.name)
        if (!wrapper.exported || !imported || imported.file !== wrapper.file) continue
      }
      const visit = (node) => {
        if (ts.isCallExpression(node)) {
          const callee = unwrap(node.expression)
          if (ts.isIdentifier(callee) && callee.text === wrapper.name && node.arguments.length > wrapper.index) {
            callSites++
            const site = where(sf, node)
            const ctx = { wrappers: [] }
            let argument = node.arguments[wrapper.index]
            if (wrapper.prop) {
              const options = unwrap(argument)
              const prop = ts.isObjectLiteralExpression(options)
                ? options.properties.find(p => p.name && p.name.getText(sf) === wrapper.prop)
                : null
              argument = prop && ts.isPropertyAssignment(prop) ? prop.initializer : prop && ts.isShorthandPropertyAssignment(prop) ? prop.name : null
            }
            const resolved = argument ? resolveNames(argument, sf, root, ctx) : null
            if (resolved === null) dynamic.push({ site, text: node.arguments[wrapper.index].getText(sf).slice(0, 80), via: wrapper.name })
            else record(resolved, site)
            wrappers.push(...ctx.wrappers.map(w => ({ ...w, origin: wrapper.origin })))
          }
        }
        ts.forEachChild(node, visit)
      }
      visit(sf)
    }
    if (callSites === 0) {
      // A parameter nothing in the frontend supplies: the name is unknown.
      dynamic.push({ ...wrapper.origin, via: wrapper.name })
    }
  }

  return { invoked, dynamic }
}

/** Files that are not part of the shipped frontend. */
function defaultExclude(relative) {
  const p = relative.split(path.sep).join('/')
  return /\.(test|spec)\.tsx?$/.test(p) || p.startsWith('website/src/components/demo/')
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

export function analyze(root = REPO_ROOT) {
  const registered = registeredCommands(root)
  const mocked = mockedCommands(root)
  const { invoked, dynamic } = invokedCommands(root)
  const sorted = (items) => [...items].sort()
  const invokedNames = new Set([...invoked.keys()].filter(n => !n.startsWith('plugin:')))
  const allowlisted = new Set(Object.keys(DELIBERATELY_UNMOCKED))
  return {
    registered: sorted(registered),
    mocked: sorted(mocked),
    invoked: sorted(invokedNames),
    callSites: Object.fromEntries(sorted(invokedNames).map(n => [n, invoked.get(n)])),
    /** Invoked by the frontend, no demo mock, not allowlisted. */
    unmocked: sorted(invokedNames).filter(n => !mocked.has(n) && !allowlisted.has(n)),
    /** Registered in Rust, never invoked by the frontend. */
    neverInvoked: sorted(registered).filter(n => !invokedNames.has(n)),
    /** Mocked, but no such command is registered. */
    staleMocks: sorted(mocked).filter(n => !registered.has(n)),
    /** Invoked by the frontend, but no such command is registered. */
    invokedUnregistered: sorted(invokedNames).filter(n => !registered.has(n)),
    /** Mocked and registered, but the frontend never invokes it. */
    mockedNeverInvoked: sorted(mocked).filter(n => registered.has(n) && !invokedNames.has(n)),
    /** Allowlist entries that are mocked after all, or no longer invoked. */
    staleAllowlist: sorted(allowlisted).filter(n => mocked.has(n) || !invokedNames.has(n)),
    /** `invoke(...)` call sites whose command name is not statically known. */
    dynamic,
  }
}

function main() {
  const args = new Set(process.argv.slice(2))
  const result = analyze()
  if (args.has('--json')) {
    process.stdout.write(JSON.stringify(result, null, 2) + '\n')
  } else {
    const section = (title, names, detail) => {
      console.log(`\n${title} (${names.length})`)
      for (const name of names) console.log(`  ${name}${detail ? detail(name) : ''}`)
    }
    const sites = args.has('--where') ? (name) => `  <- ${result.callSites[name].join(', ')}` : null
    console.log(`registered: ${result.registered.length}  invoked by frontend: ${result.invoked.length}  mocked: ${result.mocked.length}`)
    section('Invoked by the frontend but unmocked', result.unmocked, sites)
    section('Registered but never invoked by the frontend', result.neverInvoked)
    section('Mocked but not registered (stale mocks)', result.staleMocks)
    section('Invoked by the frontend but not registered', result.invokedUnregistered, sites)
    section('Mocked and registered but never invoked by the frontend', result.mockedNeverInvoked)
    section('Deliberately unmocked (allowlist)', Object.keys(DELIBERATELY_UNMOCKED).sort(), (n) => `  - ${DELIBERATELY_UNMOCKED[n]}`)
    section('Stale allowlist entries', result.staleAllowlist)
    console.log(`\nDynamic invoke call sites, command name not statically known (${result.dynamic.length})`)
    for (const d of result.dynamic) console.log(`  ${d.site}  ${d.via ? `[via ${d.via}] ` : ''}${d.text}`)
  }
  const failed = result.unmocked.length || result.staleMocks.length || result.invokedUnregistered.length ||
    result.staleAllowlist.length || result.dynamic.length
  process.exitCode = failed ? 1 : 0
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main()
