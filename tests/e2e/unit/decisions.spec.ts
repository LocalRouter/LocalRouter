import { test, expect } from '@playwright/test'
import { decisionAnswersForView, decisionQuestionsForView } from '../../../src/utils/decisions'
import { apiFromPath, llmApiFlow } from '../../../src/views/monitor/llm-api'

test('Decisions monitor preserves typed values, unnamed questions and refusals', () => {
  const questions = decisionQuestionsForView([{ type: 'choice', name: 'pick', instructions: '?', choices: [{ value: true }, { value: 'true' }] }, { type: 'predicate', instructions: '?' }])
  expect(questions['pick (1)']).toEqual({ type: 'choice', instructions: '?', criteria: { true: null, '"true"': null } })
  const answers = decisionAnswersForView([{ type: 'choice', name: 'pick', choice: true, confidence: 0.8, probabilities: [{ value: true, probability: 0.9 }, { value: 'true', probability: 0.1 }] }, { type: 'refusal', name: null }])
  expect(answers['pick (1)']).toEqual({ type: 'choice', choice: 'true', confidence: 0.8, probabilities: { true: 0.9, '"true"': 0.1 } })
  expect(answers['Question (2)']).toEqual({ type: 'refusal' })
})

test('malformed captures do not crash or invent successful answers', () => {
  expect(decisionAnswersForView([null, {}, { type: 'choice', choice: {}, confidence: 1, probabilities: [] }, { type: 'unknown' }])).toEqual({})
  expect(decisionQuestionsForView([null, {}, { type: 'choice', instructions: '?', choices: null }])).toEqual({})
  expect(decisionAnswersForView(undefined)).toEqual({})
  const q = decisionQuestionsForView([{ type: 'predicate', instructions: '?' }, { type: 'predicate', name: 'Question', instructions: '?' }])
  expect(Object.keys(q)).toHaveLength(2)
})

test('Decisions client and upstream APIs retain translation attribution', () => {
  expect(apiFromPath('/v1/decisions?x=1')).toBe('decisions')
  expect(llmApiFlow({ endpoint: '/decisions', upstream_api: 'system_one' })).toEqual({ client: 'decisions', upstream: 'system_one', translated: true })
  expect(llmApiFlow({ client_api: 'system_one', upstream_api: 'decisions' }).translated).toBe(true)
})
