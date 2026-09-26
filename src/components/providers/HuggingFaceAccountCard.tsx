import { useCallback, useEffect, useRef, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-shell"
import { toast } from "sonner"
import { ExternalLink, Eye, EyeOff, KeyRound, Loader2, LogIn, LogOut, UserCheck } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import type {
  HfAccount,
  HfSignInStart,
  HfSignInStatus,
  LocalModelsHfSetTokenParams,
  LocalModelsHfSignInFlowParams,
} from "@/types/tauri-commands"

const TOKENS_URL = "https://huggingface.co/settings/tokens"
const POLL_MS = 1500

/**
 * The account is shared by every Local Embedded provider. It is fetched once
 * per session (the backend asks Hugging Face who the token belongs to) and
 * refreshed only after the user changes it.
 */
let cachedAccount: HfAccount | null = null
const subscribers = new Set<(a: HfAccount) => void>()

function publish(account: HfAccount) {
  cachedAccount = account
  subscribers.forEach((fn) => fn(account))
}

function useHfAccount() {
  const [account, setAccount] = useState<HfAccount | null>(cachedAccount)
  useEffect(() => {
    subscribers.add(setAccount)
    if (!cachedAccount) {
      invoke<HfAccount>("local_models_hf_account")
        .then(publish)
        .catch(() => publish({ signed_in: false, method: null, username: null, expires_at: null }))
    }
    return () => {
      subscribers.delete(setAccount)
    }
  }, [])
  return account
}

function formatExpiry(unixSecs: number): string {
  return new Date(unixSecs * 1000).toLocaleString()
}

interface HuggingFaceAccountCardProps {
  /** Why this provider needs an account. */
  description?: string
}

/** Hugging Face sign-in: browser OAuth or a pasted access token. */
export function HuggingFaceAccountCard({ description }: HuggingFaceAccountCardProps) {
  const account = useHfAccount()
  const [flowId, setFlowId] = useState<string | null>(null)
  const [showToken, setShowToken] = useState(false)
  const [tokenVisible, setTokenVisible] = useState(false)
  const [token, setToken] = useState("")
  const [saving, setSaving] = useState(false)
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null)

  const stopPolling = useCallback(() => {
    if (pollRef.current) clearInterval(pollRef.current)
    pollRef.current = null
  }, [])

  useEffect(() => stopPolling, [stopPolling])

  const signIn = async () => {
    try {
      const start = await invoke<HfSignInStart>("local_models_hf_sign_in")
      setFlowId(start.flow_id)
      await open(start.auth_url)
      stopPolling()
      pollRef.current = setInterval(async () => {
        try {
          const status = await invoke<HfSignInStatus>("local_models_hf_sign_in_poll", {
            flowId: start.flow_id,
          } satisfies LocalModelsHfSignInFlowParams)
          if (status.state === "pending") return
          stopPolling()
          setFlowId(null)
          if (status.state === "success" && status.account) {
            publish(status.account)
            toast.success(
              status.account.username
                ? `Signed in to Hugging Face as ${status.account.username}`
                : "Signed in to Hugging Face",
            )
          } else if (status.state === "error") {
            toast.error(`Hugging Face sign-in failed: ${status.message ?? "unknown error"}`)
          } else if (status.state === "timeout") {
            toast.error("Hugging Face sign-in timed out")
          }
        } catch (err) {
          stopPolling()
          setFlowId(null)
          toast.error(`Hugging Face sign-in failed: ${err}`)
        }
      }, POLL_MS)
    } catch (err) {
      setFlowId(null)
      toast.error(`Could not start Hugging Face sign-in: ${err}`)
    }
  }

  const cancelSignIn = async () => {
    stopPolling()
    const id = flowId
    setFlowId(null)
    if (id) {
      await invoke("local_models_hf_sign_in_cancel", {
        flowId: id,
      } satisfies LocalModelsHfSignInFlowParams).catch(() => {})
    }
  }

  const saveToken = async () => {
    setSaving(true)
    try {
      const acct = await invoke<HfAccount>("local_models_hf_set_token", {
        token: token.trim(),
      } satisfies LocalModelsHfSetTokenParams)
      publish(acct)
      setToken("")
      setShowToken(false)
      toast.success(acct.username ? `Token saved for ${acct.username}` : "Token saved")
    } catch (err) {
      toast.error(`${err}`)
    } finally {
      setSaving(false)
    }
  }

  const signOut = async () => {
    try {
      await invoke("local_models_hf_sign_out")
      publish({ signed_in: false, method: null, username: null, expires_at: null })
      toast.success("Signed out of Hugging Face")
    } catch (err) {
      toast.error(`Could not sign out: ${err}`)
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Hugging Face account</CardTitle>
        <CardDescription>
          {description ??
            "Optional. Needed for gated models (Llama, Gemma, …) and raises download rate limits. Shared by all Local Embedded providers."}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {account === null ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" /> Checking…
          </div>
        ) : account.signed_in ? (
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <UserCheck className="h-4 w-4 text-green-600" />
            <span>
              Signed in{account.username ? <> as <span className="font-medium">{account.username}</span></> : null}
            </span>
            <Badge variant="secondary">
              {account.method === "oauth" ? "Hugging Face sign-in" : "Access token"}
            </Badge>
            {account.method === "oauth" && account.expires_at != null && (
              <span className="text-xs text-muted-foreground">
                renews after {formatExpiry(account.expires_at)}
              </span>
            )}
            <Button variant="outline" size="sm" className="ml-auto" onClick={signOut}>
              <LogOut className="mr-1 h-4 w-4" />
              Sign out
            </Button>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-2">
              {flowId ? (
                <>
                  <span className="flex items-center gap-2 text-sm text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    Finish signing in in your browser…
                  </span>
                  <Button variant="outline" size="sm" onClick={cancelSignIn}>
                    Cancel
                  </Button>
                </>
              ) : (
                <>
                  <Button size="sm" onClick={signIn}>
                    <LogIn className="mr-1 h-4 w-4" />
                    Sign in with Hugging Face
                  </Button>
                  <Button variant="outline" size="sm" onClick={() => setShowToken((v) => !v)}>
                    <KeyRound className="mr-1 h-4 w-4" />
                    Use an access token
                  </Button>
                </>
              )}
            </div>
            {showToken && !flowId && (
              <div className="space-y-2">
                <div className="flex items-center gap-2">
                  <div className="relative flex-1">
                    <Input
                      type={tokenVisible ? "text" : "password"}
                      placeholder="hf_…"
                      value={token}
                      autoComplete="off"
                      spellCheck={false}
                      onChange={(e) => setToken(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && token.trim()) saveToken()
                      }}
                      aria-label="Hugging Face access token"
                    />
                    <button
                      type="button"
                      className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground"
                      onClick={() => setTokenVisible((v) => !v)}
                      aria-label={tokenVisible ? "Hide token" : "Show token"}
                    >
                      {tokenVisible ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                    </button>
                  </div>
                  <Button size="sm" onClick={saveToken} disabled={!token.trim() || saving}>
                    {saving && <Loader2 className="mr-1 h-4 w-4 animate-spin" />}
                    Save
                  </Button>
                </div>
                <p className="text-xs text-muted-foreground">
                  Create a fine-grained token with read access to public gated repositories at{" "}
                  <button
                    type="button"
                    className="inline-flex items-center gap-1 underline-offset-2 hover:underline"
                    onClick={() => open(TOKENS_URL)}
                  >
                    huggingface.co/settings/tokens <ExternalLink className="h-3 w-3" />
                  </button>
                  . The token is stored in your system keychain.
                </p>
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
