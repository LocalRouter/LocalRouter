"""Offline reproducibility and RE2-to-Rust rule-import regression checks."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('secret_rules',ROOT/'scripts/update-secret-rules.py')
module=importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class SecretRulesTests(unittest.TestCase):
    def test_pinned_snapshot_is_complete_and_reproducible(self):
        generated=module.generate()
        self.assertEqual(json.loads((module.DATA/'rules.json').read_text()),generated)
        self.assertEqual(generated['upstream_rule_count'],len(generated['rules'])+len(generated['excluded']))
        self.assertGreaterEqual(len(generated['rules']),200)
        ids=[rule['id'] for rule in generated['rules']]
        self.assertEqual(len(ids),len(set(ids)))
        for rule in generated['rules']:
            self.assertTrue(rule['regex'])
            self.assertTrue(rule['keywords'])
        self.assertTrue(all(item['reason'] for item in generated['excluded']))

    def test_ascii_classes_boundaries_and_literal_braces_preserve_go_meaning(self):
        self.assertEqual(module.ascii_regex(r'\b([\w.-]+)\s+\d{2,4}\B'),r'(?-u:\b)([A-Za-z0-9_.-]+)[ \t\n\f\r]+[0-9]{2,4}(?-u:\B)')
        self.assertEqual(module.ascii_regex(r'^\$(?:\d+|{\d+})$'),r'^\$(?:[0-9]+|\{[0-9]+\})$')
        self.assertEqual(module.ascii_regex(r'\p{L}\x{20}\w{8}'),r'\p{L}\x{20}[A-Za-z0-9_]{8}')
        self.assertEqual(module.ascii_regex(r'\\w'),r'\\w')
        self.assertEqual(module.ascii_regex(r'(?i)^true|false|null$'),r'(?i)^(?:true|false|null)$')

    def test_source_tampering_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            data=Path(directory)
            for name in ['upstream.json','gitleaks.toml','LICENSE']:
                (data/name).write_bytes((module.DATA/name).read_bytes())
            (data/'LICENSE').write_text('modified')
            with self.assertRaisesRegex(ValueError,'checksum mismatch'):module.generate(data)

    def test_new_allowlist_semantics_require_explicit_review(self):
        with self.assertRaises(ValueError):module.allowlist({'unknown':True})
        converted=module.allowlist({'condition':'AND','paths':['source.tf'],'regexTarget':'line','regexes':['example']})
        self.assertTrue(converted['requires_path_or_commit'])
        self.assertEqual(converted['condition'],'AND')

if __name__=='__main__': unittest.main()
