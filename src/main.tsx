import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App'
import { FirewallApproval } from './views/firewall-approval'

// Check if this is a firewall approval popup window
const isFirewallApproval = (window as any).__TAURI_INTERNALS__?.metadata?.currentWebview?.label?.startsWith('firewall-approval-')

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {isFirewallApproval ? <FirewallApproval /> : <App />}
  </StrictMode>,
)
