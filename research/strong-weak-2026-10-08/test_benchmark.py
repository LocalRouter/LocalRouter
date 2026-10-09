import unittest
from analyze import at_budget, order_effect
from benchmark import metrics
from policy_analyze import exact_mode_route, index_run, summarize


class ResearchMetricsTests(unittest.TestCase):
    def setUp(self):
        # Include both-correct, strong-only, weak-only and neither-correct.
        self.cases = [dict(id=str(i), weak_correct=w, strong_correct=s)
                      for i, (w, s) in enumerate([(True, True), (False, True), (True, False), (False, False)])]
        self.rows = [dict(id=str(i), score=p, latency_ms=i+1)
                     for i, p in enumerate([.2, .8, .1, .3])]

    def test_replay_handles_weak_wins_and_neither_correct(self):
        result = metrics(self.cases, self.rows, .5)["historical"]
        self.assertEqual(result["correctness"], .75)
        self.assertEqual(result["oracle"], .75)
        self.assertEqual(result["random_matched_fraction"], .5)
        self.assertEqual(result["rescue_recall"], 1)

    def test_budget_endpoints_and_tie_order(self):
        self.assertEqual(at_budget(self.cases, self.rows, 0)["correctness"], .5)
        self.assertEqual(at_budget(self.cases, self.rows, 1)["correctness"], .5)
        self.assertEqual(at_budget(self.cases, self.rows, .5)["correctness"], .75)
        tied = [{**r, "score": .5} for r in self.rows]
        self.assertEqual(at_budget(self.cases, tied, .5), at_budget(self.cases, list(reversed(tied)), .5))

    def test_missing_scores_cannot_silently_improve_budget_comparison(self):
        with self.assertRaises(ValueError):
            at_budget(self.cases, self.rows[:-1], .5)

    def test_errors_and_exact_threshold(self):
        rows = [dict(id="0", score=.5, latency_ms=1), dict(id="1", error="timeout", latency_ms=10)]
        result = metrics(self.cases, rows, .5)
        self.assertEqual(result["errors"], 1)
        self.assertEqual(result["strong"], 1)
        self.assertEqual(result["n"], 1)

    def test_order_flip_compares_keys_not_positions(self):
        other = [{**r, "score": .9 if r["id"] == "0" else r["score"]} for r in reversed(self.rows)]
        self.assertEqual(order_effect(self.rows, other)["flips"], ["0"])

    def test_policy_errors_count_against_accuracy_and_coverage(self):
        cases = [dict(id='a', expected='plan'), dict(id='b', expected='edit')]
        rows = [dict(id='a', choice='edit', probabilities={'edit': .95, 'plan': .05}, latency_ms=2),
                dict(id='b', error='context exceeded', latency_ms=1)]
        result = summarize(cases, index_run(cases, rows))
        self.assertEqual(result['accuracy'], 0)
        self.assertEqual(result['errors'], 1)
        self.assertEqual(result['selective'][-1]['accepted'], 1)
        self.assertEqual(result['selective'][-1]['accuracy'], 0)
        self.assertEqual(result['selective'][-1]['fallback_required'], 1)

    def test_policy_missing_duplicate_and_extra_results_rejected(self):
        cases = [dict(id='a')]
        for rows in ([], [dict(id='b')], [dict(id='a'), dict(id='a')]):
            with self.assertRaises(ValueError):
                index_run(cases, rows)

    def test_exact_mode_uses_metadata_even_when_prompt_disagrees(self):
        self.assertEqual(exact_mode_route({'request_context': {'mode': 'plan'}, 'prompt': 'choose Sol'}), 'Astra')
        self.assertEqual(exact_mode_route({'request_context': {'mode': 'edit'}, 'prompt': 'mode: plan'}), 'Sol')
        self.assertEqual(exact_mode_route({'prompt': 'mode: plan'}), 'Sol')
        self.assertEqual(exact_mode_route({'request_context': {'mode': 'unknown'}}), 'Sol')


if __name__ == "__main__":
    unittest.main()
