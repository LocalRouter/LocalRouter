import type {
  ReplayArtifact,
  ReplayArtifactKind,
  ReplayArtifactList,
  ReplayDataset,
  ReplayRun,
  ReplayRunSummary,
  ReplayResult,
  ReplayRunRequest,
  ReplayEstimate,
  RoutingPolicy,
} from '@app/types/tauri-commands'
const artifacts = new Map<string, ReplayArtifact>()
const runs = new Map<string, ReplayRun>()
function getRun(id: string): ReplayRun {
  const run = runs.get(id)
  if (!run) throw Error('Run not found')
  return run
}
function summary(run: ReplayRun): ReplayRunSummary {
  return {
    ...run,
    results: run.results.map((result, index) => ({ ...result, index })),
  }
}
function read(kind: ReplayArtifactKind, id: string): ReplayArtifact {
  const a = artifacts.get(id)
  if (!a || a.summary.kind !== kind) throw Error('Artifact not found')
  return structuredClone(a)
}
function save(
  value: ReplayDataset | ReplayRun,
  kind: ReplayArtifactKind,
): ReplayArtifact {
  const old = artifacts.get(value.id)
  if ((old?.summary.revision ?? 0) !== value.revision)
    throw Error('Artifact changed; reload before saving')
  const next = structuredClone(value)
  next.revision++
  const result: ReplayArtifact = {
    schema_version: 1,
    summary: {
      id: next.id,
      name: next.name,
      kind,
      revision: next.revision,
      saved_at: new Date().toISOString(),
      case_count: next.cases.length,
      result_count: 'results' in next ? next.results.length : 0,
      client_id: 'client_id' in next ? next.client_id : null,
    },
    value: next,
  }
  artifacts.set(value.id, result)
  return structuredClone(result)
}
export const mockReplay = {
  replay_import_run: (args: { json: string }): ReplayRunSummary => {
    const data = JSON.parse(args.json)
    const run = (data.value ?? data) as ReplayRun
    if (!Array.isArray(run.results)) throw Error('Import needs results')
    run.id = crypto.randomUUID()
    run.revision = 0
    run.dataset_id = null
    if (run.status === 'running') run.status = 'interrupted'
    run.stop_reason = 'Imported snapshot; no inference resumed'
    runs.set(run.id, run)
    return summary(run)
  },
  replay_preview: (args: { request: ReplayRunRequest }): ReplayEstimate[] =>
    args.request.cases.flatMap((c) =>
      args.request.targets.map((t) => ({
        case_id: c.id,
        target: t.kind === 'model' ? t.model : t.name,
        estimated_usd: 0,
        warnings: ['demo_only_no_inference'],
      })),
    ),
  replay_list: (): ReplayArtifactList => ({
    items: [...artifacts.values()].map((a) => a.summary),
    issues: [],
  }),
  replay_read: (args: {
    kind: ReplayArtifactKind
    id: string
  }): ReplayArtifact => read(args.kind, args.id),
  replay_save_dataset: (args: { dataset: ReplayDataset }): ReplayArtifact =>
    save(args.dataset, 'dataset'),
  replay_delete: (args: {
    kind: ReplayArtifactKind
    id: string
    revision: number
  }): void => {
    const a = read(args.kind, args.id)
    if (a.summary.revision !== args.revision) throw Error('Artifact changed')
    artifacts.delete(args.id)
    runs.delete(args.id)
  },
  replay_start: (args: { request: ReplayRunRequest }): string => {
    const request = args.request
    const id = crypto.randomUUID()
    const results: ReplayResult[] = request.cases.flatMap((c) =>
      request.targets.map((target) => ({
        case_id: c.id,
        case_name: c.name,
        target: target.kind === 'model' ? target.model : target.name,
        status: 'success',
        http_request_sent: false,
        http_status: 200,
        latency_ms: 40,
        input_tokens: 12,
        output_tokens: 8,
        cost: {
          reported_usd: 0,
          estimated_usd: null,
          unknown_calls: 0,
          classifier_reported_usd: null,
          classifier_estimated_usd: null,
          classifier_cost_unknown: false,
        },
        checks: [],
        request: { demo: true },
        response: {
          choices: [
            {
              message: {
                role: 'assistant',
                content: 'Demo response. No inference was sent.',
              },
            },
          ],
        },
        judge: null,
        error: null,
        policy_snapshot: { demo: true },
        config_fingerprint: 'demo',
        policy_fingerprint: 'demo',
      })),
    )
    runs.set(id, {
      id,
      revision: 0,
      dataset_id: request.dataset_id,
      name: 'Demo comparison',
      client_id: request.client_id,
      started_at: new Date().toISOString(),
      finished_at: new Date().toISOString(),
      stop_reason: null,
      status: 'completed',
      cases: request.cases,
      targets: request.targets,
      results,
      judge_of: null,
      judge_model: null,
      judge_rubric: null,
    })
    return id
  },
  replay_runs: (): ReplayRunSummary[] => [...runs.values()].map(summary),
  replay_status: (args: { id: string }): ReplayRunSummary =>
    summary(getRun(args.id)),
  replay_result: (args: { id: string; index: number }): ReplayResult => {
    const result = getRun(args.id).results[args.index]
    if (!result) throw Error('Result not found')
    return result
  },
  replay_get_run: (args: { id: string }): ReplayRun => getRun(args.id),
  replay_cancel: (args: { id: string }): void => {
    getRun(args.id).status = 'cancelled'
  },
  replay_save_run: (args: { id: string }): ReplayArtifact => {
    const result = save(getRun(args.id), 'run')
    getRun(args.id).revision = result.summary.revision
    return result
  },
  replay_judge: (): string => {
    throw Error('Paid judge inference is unavailable in the website demo')
  },
  replay_current_policy: (): RoutingPolicy | null => null,
  replay_import: (args: { json: string }): ReplayDataset => {
    const value = JSON.parse(args.json)
    const source = value.value ?? value
    if (!Array.isArray(source.cases) || !source.cases.length)
      throw Error('Import needs cases')
    return {
      id: crypto.randomUUID(),
      revision: 0,
      saved_at: null,
      name: source.name ?? 'Imported dataset',
      cases: source.cases,
    }
  },
}
