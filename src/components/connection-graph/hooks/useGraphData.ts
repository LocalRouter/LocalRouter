import { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { Client, Provider, McpServer, Skill, HealthCacheState, UseGraphDataResult } from '../types'

// How long to show a client as "connected" after last activity (ms)
const ACTIVITY_TIMEOUT_MS = 10_000

export function useGraphData(): UseGraphDataResult {
  const [clients, setClients] = useState<Client[]>([])
  const [providers, setProviders] = useState<Provider[]>([])
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [skills, setSkills] = useState<Skill[]>([])
  const [healthState, setHealthState] = useState<HealthCacheState | null>(null)
  const [activeConnections, setActiveConnections] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  // Track last activity time per client (client_id -> timestamp)
  const lastActivityRef = useRef<Map<string, number>>(new Map())
  // Track SSE connections (persistent until closed)
  const sseConnectionsRef = useRef<Set<string>>(new Set())

  // Compute active connections from SSE connections + recent activity
  const computeActiveConnections = useCallback(() => {
    const now = Date.now()
    const active = new Set<string>()

    // Add all active SSE connections
    sseConnectionsRef.current.forEach(id => active.add(id))

    // Add clients with recent activity
    lastActivityRef.current.forEach((timestamp, clientId) => {
      if (now - timestamp < ACTIVITY_TIMEOUT_MS) {
        active.add(clientId)
      }
    })

    setActiveConnections(Array.from(active))
  }, [])

  // Record client activity
  const recordActivity = useCallback((clientId: string) => {
    lastActivityRef.current.set(clientId, Date.now())
    computeActiveConnections()
  }, [computeActiveConnections])

  // Fetch all data
  const fetchData = useCallback(async () => {
    try {
      const [clientList, providerList, mcpServerList, skillList, health, connections] = await Promise.all([
        invoke<Client[]>('list_clients').catch(() => []),
        invoke<Provider[]>('list_provider_instances').catch(() => []),
        invoke<McpServer[]>('list_mcp_servers').catch(() => []),
        invoke<Skill[]>('list_skills').catch(() => []),
        invoke<HealthCacheState>('get_health_cache').catch(() => null),
        invoke<string[]>('get_active_connections').catch(() => []),
      ])

      setClients(clientList)
      setProviders(providerList)
      setMcpServers(mcpServerList)
      setSkills(skillList)
      setHealthState(health)

      // Initialize SSE connections from server
      sseConnectionsRef.current = new Set(connections)
      computeActiveConnections()

      setError(null)
    } catch (err) {
      console.error('Failed to fetch graph data:', err)
      setError(err instanceof Error ? err.message : 'Failed to fetch data')
    } finally {
      setLoading(false)
    }
  }, [computeActiveConnections])

  // Initial fetch
  useEffect(() => {
    fetchData()
  }, [fetchData])

  // Set up interval to expire old activity
  useEffect(() => {
    const interval = setInterval(() => {
      const now = Date.now()
      let hasExpired = false

      lastActivityRef.current.forEach((timestamp, clientId) => {
        if (now - timestamp >= ACTIVITY_TIMEOUT_MS) {
          lastActivityRef.current.delete(clientId)
          hasExpired = true
        }
      })

      if (hasExpired) {
        computeActiveConnections()
      }
    }, 1000) // Check every second

    return () => clearInterval(interval)
  }, [computeActiveConnections])

  // Subscribe to events
  useEffect(() => {
    const unlisteners: Array<() => void> = []

    // Health status changes
    const setupHealthListener = async () => {
      const unlisten = await listen<HealthCacheState>('health-status-changed', (event) => {
        setHealthState(event.payload)
      })
      unlisteners.push(unlisten)
    }

    // SSE connection opened (persistent connection)
    const setupConnectionOpenedListener = async () => {
      const unlisten = await listen<string>('sse-connection-opened', (event) => {
        sseConnectionsRef.current.add(event.payload)
        computeActiveConnections()
      })
      unlisteners.push(unlisten)
    }

    // SSE connection closed - but keep showing as active for 10 more seconds
    const setupConnectionClosedListener = async () => {
      const unlisten = await listen<string>('sse-connection-closed', (event) => {
        sseConnectionsRef.current.delete(event.payload)
        // Record activity to keep showing for 10 more seconds
        recordActivity(event.payload)
      })
      unlisteners.push(unlisten)
    }

    // Client activity (HTTP requests)
    const setupActivityListener = async () => {
      const unlisten = await listen<string>('client-activity', (event) => {
        console.log('[ConnectionGraph] client-activity event:', event.payload)
        recordActivity(event.payload)
      })
      unlisteners.push(unlisten)
    }

    // Config changes (clients, providers, MCP servers updated)
    const setupConfigListener = async () => {
      const unlisten = await listen('config-changed', () => {
        fetchData()
      })
      unlisteners.push(unlisten)
    }

    // Clients changed
    const setupClientsListener = async () => {
      const unlisten = await listen('clients-changed', () => {
        fetchData()
      })
      unlisteners.push(unlisten)
    }

    // Skills changed
    const setupSkillsListener = async () => {
      const unlisten = await listen('skills-changed', () => {
        fetchData()
      })
      unlisteners.push(unlisten)
    }

    // Set up all listeners
    Promise.all([
      setupHealthListener(),
      setupConnectionOpenedListener(),
      setupConnectionClosedListener(),
      setupActivityListener(),
      setupConfigListener(),
      setupClientsListener(),
      setupSkillsListener(),
    ])

    return () => {
      unlisteners.forEach(unlisten => unlisten())
    }
  }, [fetchData, computeActiveConnections, recordActivity])

  return {
    clients,
    providers,
    mcpServers,
    skills,
    healthState,
    activeConnections,
    loading,
    error,
  }
}
