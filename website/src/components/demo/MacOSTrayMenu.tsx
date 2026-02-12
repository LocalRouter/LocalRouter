/**
 * macOS Tray Menu component for website demo
 *
 * !! SYNC WITH: src-tauri/src/ui/tray_menu.rs !!
 * When tray_menu.rs changes, update this component to match.
 *
 * Key sync points:
 * - Menu item labels and icons (TRAY_INDENT, ICON_PAD patterns)
 * - Menu structure (headers, separators, submenus)
 * - Client submenu structure (Copy ID, strategies, MCP, skills)
 */

import { useState } from 'react'
import { ChevronRight, Copy, Settings, ExternalLink } from 'lucide-react'
import { mockData } from './mockData'

// Unicode spacing from tray_menu.rs (keep in sync!)
// TRAY_INDENT = '\u{2003}\u{2009}\u{2009}' (em-space + 2 thin spaces)
// ICON_PAD = '\u{2009}\u{2009}' (2 thin spaces per side)
const TRAY_INDENT = '\u2003\u2009\u2009'

interface MacOSTrayMenuProps {
  onClose: () => void
}

function MenuItem({
  icon,
  label,
  onClick,
  disabled = false,
}: {
  icon?: React.ReactNode
  label: string
  onClick?: () => void
  disabled?: boolean
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`w-full px-3 py-1 text-left text-[13px] flex items-center gap-2 ${
        disabled
          ? 'text-gray-400 cursor-default'
          : 'text-gray-800 hover:bg-blue-500 hover:text-white'
      }`}
    >
      {icon && <span className="w-4 text-center">{icon}</span>}
      <span>{label}</span>
    </button>
  )
}

function Separator() {
  return <div className="my-1 border-t border-gray-300/50" />
}

const MAX_TRAY_ITEMS = 10

function ClientSubmenu({ client }: { client: (typeof mockData.clients)[0] }) {
  const showLlm = client.client_mode !== 'mcp_only'
  const showMcp = client.client_mode !== 'llm_only'

  return (
    <div className="py-1">
      {/* Client name header (disabled) */}
      <MenuItem label={client.name} disabled />

      {/* Enable/disable toggle */}
      <MenuItem
        label={client.enabled ? '● Enabled' : '○ Disabled'}
      />

      {/* Copy actions */}
      <MenuItem
        icon={<Copy className="w-3 h-3" />}
        label="Copy Client ID (OAuth)"
        onClick={() => {
          navigator.clipboard.writeText(client.client_id)
        }}
      />
      <MenuItem
        icon={<Copy className="w-3 h-3" />}
        label="Copy API Key / Client Secret"
        onClick={() => {
          navigator.clipboard.writeText('demo-api-key-' + client.id)
        }}
      />

      {/* Model strategy section (hidden for mcp_only) */}
      {showLlm && (
        <>
          <Separator />

          <MenuItem label="Model strategy" disabled />

          {mockData.strategies.slice(0, MAX_TRAY_ITEMS).map((strategy) => {
            const isSelected = strategy.id === client.strategy_id
            return (
              <MenuItem
                key={strategy.id}
                label={isSelected ? `✓  ${strategy.name}` : `${TRAY_INDENT}${strategy.name}`}
                disabled={isSelected}
              />
            )
          })}

          {mockData.strategies.length > MAX_TRAY_ITEMS && (
            <MenuItem label={`${TRAY_INDENT}More…`} />
          )}
        </>
      )}

      {/* MCP Allowlist section (hidden for llm_only) */}
      {showMcp && (
        <>
          <Separator />

          <MenuItem label="MCP Allowlist" disabled />

          {mockData.mcpServers.length === 0 ? (
            <MenuItem label={`${TRAY_INDENT}No MCPs configured`} disabled />
          ) : (
            <>
              {mockData.mcpServers.slice(0, MAX_TRAY_ITEMS).map((server) => {
                const serverPerm = client.mcp_permissions.servers[server.id]
                const isAllowed =
                  serverPerm === 'allow' ||
                  (serverPerm === undefined && client.mcp_permissions.global === 'allow')
                return (
                  <MenuItem
                    key={server.id}
                    label={isAllowed ? `✓  ${server.name}` : `${TRAY_INDENT}${server.name}`}
                  />
                )
              })}

              {mockData.mcpServers.length > MAX_TRAY_ITEMS && (
                <MenuItem label={`${TRAY_INDENT}More…`} />
              )}
            </>
          )}

          <Separator />

          <MenuItem label="Skills Allowlist" disabled />

          {mockData.skills.length === 0 ? (
            <MenuItem label={`${TRAY_INDENT}No Skills configured`} disabled />
          ) : (
            <>
              {mockData.skills.slice(0, MAX_TRAY_ITEMS).map((skill) => {
                const skillPerm = client.skills_permissions.skills[skill.name]
                const isAllowed =
                  skillPerm === 'allow' ||
                  (skillPerm === undefined && client.skills_permissions.global === 'allow')
                return (
                  <MenuItem
                    key={skill.name}
                    label={isAllowed ? `✓  ${skill.name}` : `${TRAY_INDENT}${skill.name}`}
                  />
                )
              })}

              {mockData.skills.length > MAX_TRAY_ITEMS && (
                <MenuItem label={`${TRAY_INDENT}More…`} />
              )}
            </>
          )}
        </>
      )}
    </div>
  )
}

function SubmenuItem({
  label,
  isOpen,
  onToggle,
  children,
}: {
  label: string
  isOpen: boolean
  onToggle: () => void
  children: React.ReactNode
}) {
  return (
    <div className="relative">
      <button
        onClick={onToggle}
        className="w-full px-3 py-1 text-left text-[13px] flex items-center justify-between text-gray-800 hover:bg-blue-500 hover:text-white"
      >
        <span>{TRAY_INDENT}{label}</span>
        <ChevronRight className="w-3 h-3" />
      </button>
      {isOpen && (
        <div className="absolute left-full top-0 ml-1 w-64 rounded-md bg-gray-100/95 backdrop-blur-xl shadow-xl border border-gray-300/50">
          {children}
        </div>
      )}
    </div>
  )
}

export function MacOSTrayMenu({ onClose }: MacOSTrayMenuProps) {
  const [openSubmenu, setOpenSubmenu] = useState<string | null>(null)

  return (
    <>
      {/* Backdrop to close on outside click */}
      <div className="fixed inset-0 z-40" onClick={onClose} />

      <div className="absolute right-4 top-7 z-50 w-64 rounded-md bg-gray-100/95 backdrop-blur-xl shadow-xl border border-gray-300/50 py-1 text-[13px]">
        {/* Header - sync with tray_menu.rs */}
        <div className="px-3 py-1 text-gray-400 cursor-default text-[13px]">
          LocalRouter on {mockData.serverConfig.host}:{mockData.serverConfig.port}
        </div>

        <Separator />

        {/* Settings */}
        <MenuItem
          icon={<Settings className="w-3 h-3" />}
          label="Settings..."
          onClick={onClose}
        />

        {/* Copy URL */}
        <MenuItem
          icon={<ExternalLink className="w-3 h-3" />}
          label="Copy URL"
          onClick={() => {
            navigator.clipboard.writeText(
              `http://${mockData.serverConfig.host}:${mockData.serverConfig.port}`
            )
          }}
        />

        <Separator />

        {/* Clients header */}
        <div className="px-3 py-1 text-gray-400 cursor-default text-[13px]">
          Clients
        </div>

        {/* Client submenus */}
        {mockData.clients.map((client) => (
          <SubmenuItem
            key={client.id}
            label={client.name}
            isOpen={openSubmenu === client.id}
            onToggle={() =>
              setOpenSubmenu(openSubmenu === client.id ? null : client.id)
            }
          >
            <ClientSubmenu client={client} />
          </SubmenuItem>
        ))}

        <Separator />

        {/* Add client */}
        <MenuItem icon="+" label="Add && Copy Key" />

        <Separator />

        {/* Quit */}
        <MenuItem icon="⏻" label="Quit" />
      </div>
    </>
  )
}
