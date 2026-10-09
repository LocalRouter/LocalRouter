#!/usr/bin/env python3
"""Summarize fixed policy runs, counting rejected requests as unsuccessful."""
import collections
import json
import math
import statistics
from benchmark import HERE, read_jsonl


def index_run(cases, rows):
    ids = {c['id'] for c in cases}
    by_id = {r['id']: r for r in rows}
    if len(ids) != len(cases) or len(by_id) != len(rows) or set(by_id) != ids:
        raise ValueError('Require exactly one result for every case')
    return by_id


def summarize(cases, by_id):
    pairs = [(c, by_id[c['id']]) for c in cases]
    valid = [(c, r) for c, r in pairs if 'error' not in r]
    times = sorted(r['latency_ms'] for _, r in valid)
    correct = sum(r.get('choice') == c['expected'] for c, r in pairs)
    labels = collections.Counter(c['expected'] for c in cases)
    result = {'total': len(cases), 'correct': correct, 'accuracy': correct / len(cases),
              'errors': len(pairs) - len(valid), 'majority_baseline_correct': max(labels.values()),
              'confusion': dict(collections.Counter(c['expected'] + ' -> ' + r.get('choice', 'ERROR') for c, r in pairs)),
              'failures': [c['id'] for c, r in pairs if r.get('choice') != c['expected']]}
    if times:
        result['warm_success_latency_ms'] = {'p50': statistics.median(times), 'p95': times[math.ceil(.95 * len(times))-1], 'max': max(times)}
    # Descriptive coverage only: neither a fitted threshold nor validated deployment calibration.
    result['selective'] = []
    for threshold in (.5, .7, .9):
        accepted = [(c, r) for c, r in valid if max(r['probabilities'].values()) >= threshold]
        n = len(accepted)
        k = sum(c['expected'] == r['choice'] for c, r in accepted)
        result['selective'].append({'min_top_probability': threshold, 'accepted': n,
                                    'correct': k, 'accuracy': k / n if n else None,
                                    'fallback_required': len(cases) - n})
    return result


def changes(a, b):
    valid = sorted(i for i in a if 'error' not in a[i] and 'error' not in b[i])
    return {'valid_pairs': len(valid), 'changed_choices': [i for i in valid if a[i]['choice'] != b[i]['choice']]}


def exact_mode_route(state):
    """The user's exact rule on already-normalized metadata; no extraction implied."""
    return 'Astra' if state.get('request_context', {}).get('mode') == 'plan' else 'Sol'


def analyze():
    cases = read_jsonl(HERE / 'policy-cases.jsonl')
    policies = sorted({c['policy'] for c in cases})
    runs = {}
    output = {}
    for name in ('laya', 'kev', 'laya-reversed', 'kev-reversed', 'kev-latest'):
        runs[name] = index_run(cases, read_jsonl(HERE / ('policy-' + name + '.jsonl')))
        output[name] = {'all': summarize(cases, runs[name]),
                        'policies': {p: summarize([c for c in cases if c['policy'] == p], runs[name]) for p in policies},
                        'history_cases': summarize([c for c in cases if c['kind'] == 'history'], runs[name])}
    for name in ('laya', 'kev'):
        output[name]['option_order'] = changes(runs[name], runs[name + '-reversed'])
    output['kev']['latest_only_ablation'] = changes(runs['kev'], runs['kev-latest'])
    mode = [c for c in cases if c['policy'] == 'mode']
    output['exact_metadata_rule'] = {'total': len(mode), 'correct': sum(exact_mode_route(c['state']) == c['expected'] for c in mode),
                                     'scope': 'Synthetic already-normalized mode; not client metadata extraction or detection.'}
    (HERE / 'policy-summary.json').write_text(json.dumps(output, indent=2) + '\n')
    return output


if __name__ == '__main__':
    analyze()
