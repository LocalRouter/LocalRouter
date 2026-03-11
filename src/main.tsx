import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App'
import { FirewallApproval } from './views/firewall-approval'
import { SamplingApproval } from './views/sampling-approval'
import { ElicitationForm } from './views/elicitation-form'

// Check window label to determine which view to render
const windowLabel = (window as any).__TAURI_INTERNALS__?.metadata?.currentWebview?.label || ''
const isFirewallApproval = windowLabel.startsWith('firewall-approval-')
const isSamplingApproval = windowLabel.startsWith('sampling-approval-')
const isElicitationForm = windowLabel.startsWith('elicitation-form-')

function RootView() {
  if (isFirewallApproval) return <FirewallApproval />
  if (isSamplingApproval) return <SamplingApproval />
  if (isElicitationForm) return <ElicitationForm />
  return <App />
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RootView />
  </StrictMode>,
)
