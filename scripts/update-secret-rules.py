#!/usr/bin/env python3
"""Build the offline Gitleaks rule snapshot; --check verifies reproducibility.

Refresh the pinned source/metadata described in data/gitleaks/README.md before
running this script. This command never makes network requests.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / 'crates/lr-secret-scanner/data/gitleaks'
# Inference from content cannot apply file-name/commit predicates. Keep those
# rules excluded rather than dropping a predicate and broadening the match.
OVERRIDES = {
    'aws-access-token': 'AWS access-key IDs identify credentials but do not authenticate alone.',
    'generic-api-key': 'LocalRouter uses context-aware assignment rules and descriptive-name filters.',
    'private-key': 'LocalRouter handles complete/truncated armor and nested escaping.',
    'jwt': 'LocalRouter validates decoded headers and measures signature entropy.',
    'mailgun-pub-key': 'Public validation keys are not private credentials.',
    'twilio-api-key': 'The SK-prefixed API key SID is an identifier, not its authentication secret.',
}


def ascii_regex(pattern):
    """Preserve Go/RE2 ASCII Perl classes and word boundaries in Rust regex."""
    # Anchor the complete boolean-placeholder alternation. A credential that
    # happens to contain "false" is not a boolean literal.
    if pattern == r'(?i)^true|false|null$':
        pattern = r'(?i)^(?:true|false|null)$'
    classes = {'w': 'A-Za-z0-9_', 'd': '0-9', 's': r' \t\n\f\r'}
    out = []
    inside = False
    at = 0
    while at < len(pattern):
        char = pattern[at]
        if char == '\\' and at + 1 < len(pattern):
            code = pattern[at + 1]
            if code in 'pPx' and pattern[at+2:at+3] == '{':
                end = pattern.index('}', at + 3) + 1
                out.append(pattern[at:end])
                at = end
                continue
            if code.lower() in classes:
                cls = classes[code.lower()]
                if inside and code.isupper():
                    raise ValueError('Unsupported negated Perl class inside character class')
                out.append(cls if inside else '[' + ('^' if code.isupper() else '') + cls + ']')
            elif code in 'bB' and not inside:
                out.append('(?-u:\\' + code + ')')
            else:
                out.append(pattern[at:at+2])
            at += 2
            continue
        if not inside and char == '{':
            quantifier = re.match(r'\{\d+(?:,\d*)?\}', pattern[at:])
            if quantifier:
                out.append(quantifier.group())
                at += len(quantifier.group())
                continue
            out.append(r'\{')
            at += 1
            continue
        if not inside and char == '}':
            out.append(r'\}')
            at += 1
            continue
        if char == '[':
            inside = True
        elif char == ']':
            inside = False
        out.append(char)
        at += 1
    return ''.join(out)


def allowlist(raw):
    supported = {'description', 'condition', 'regexTarget', 'regexes', 'stopwords', 'paths', 'commits'}
    if set(raw) - supported:
        raise ValueError(f'Unknown allowlist fields: {set(raw) - supported}')
    return {'condition': raw.get('condition', 'OR'), 'target': raw.get('regexTarget', 'secret'),
            'regexes': [ascii_regex(p) for p in raw.get('regexes', [])],
            'stopwords': raw.get('stopwords', []),
            'requires_path_or_commit': bool(raw.get('paths') or raw.get('commits'))}


def generate(data=DATA, root=ROOT):
    metadata = json.loads((data/'upstream.json').read_text())
    for name, field in [('gitleaks.toml', 'config_sha256'), ('LICENSE', 'license_sha256')]:
        if hashlib.sha256((data/name).read_bytes()).hexdigest() != metadata[field]:
            raise ValueError(f'Pinned {name} checksum mismatch')
    config = tomllib.loads((data/'gitleaks.toml').read_text())
    local = set(re.findall(r'id: "([^"]+)"', (root/'crates/lr-secret-scanner/src/patterns/builtin.rs').read_text()))
    excluded, rules = [], []
    supported = {'id', 'description', 'regex', 'path', 'entropy', 'secretGroup', 'keywords', 'allowlists'}
    for rule in config['rules']:
        if set(rule) - supported:
            raise ValueError(f"Unknown rule fields for {rule['id']}: {set(rule)-supported}")
        reason = OVERRIDES.get(rule['id'])
        if rule['id'] in local:
            reason = 'LocalRouter format-specific rule preserves escaping, validation and compatibility.'
        if rule.get('path') or not rule.get('regex'):
            reason = 'Requires file-path metadata unavailable on outbound model text.'
        if reason:
            excluded.append({'id': rule['id'], 'reason': reason})
            continue
        rules.append({'id': rule['id'], 'description': rule['description'],
                      'regex': ascii_regex(rule['regex']), 'source_regex': rule['regex'],
                      'secret_group': rule.get('secretGroup'), 'whole_match': rule['id'] == 'jwt-base64',
                      'entropy': rule.get('entropy', 0), 'keywords': rule.get('keywords', []),
                      'allowlists': [allowlist(a) for a in rule.get('allowlists', [])]})
    return dict(metadata, license=(data/'LICENSE').read_text(), upstream_rule_count=len(config['rules']), rules=rules, excluded=excluded,
                allowlists=[allowlist(config['allowlist'])] if config.get('allowlist') else [])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    rendered = json.dumps(generate(), ensure_ascii=False, indent=2) + '\n'
    output = DATA/'rules.json'
    if args.check:
        if not output.exists() or output.read_text() != rendered:
            raise SystemExit('Bundled Gitleaks rules are stale; run scripts/update-secret-rules.py')
    else:
        output.write_text(rendered)
    snapshot = json.loads(rendered)
    print(f"Gitleaks {snapshot['version']}: {len(snapshot['rules'])} embedded rules, {len(snapshot['excluded'])} documented exclusions")

if __name__ == '__main__':
    main()
