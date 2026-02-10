/**
 * ClientCreationWizard
 *
 * Multi-step wizard for creating a new client with guided setup.
 * Steps flow dynamically based on template and mode selection:
 * 0. Welcome - Introduction to LocalRouter (first launch only)
 * 1. Template - Select an app template (Claude Code, Cursor, etc.)
 * 2. Mode - Select client mode (LLM / MCP / Both) — skipped for Custom template
 * 3. Name - Choose a name for the client
 * 4. Models - Select which models the client can access — skipped for mcp_only
 * 5. MCP - Select which MCP servers the client can access — skipped for llm_only
 * 6. Skills - Select which skills the client can access — skipped for llm_only
 * 7. Credentials - View and copy the generated credentials
 */

import { useState, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { ChevronLeft, ChevronRight, Loader2 } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/Modal"
import { Button } from "@/components/ui/Button"
import { StepWelcome } from "./steps/StepWelcome"
import { StepTemplate } from "./steps/StepTemplate"
import { StepMode } from "./steps/StepMode"
import { StepName } from "./steps/StepName"
import { StepModels } from "./steps/StepModels"
import { StepMcp } from "./steps/StepMcp"
import { StepSkills } from "./steps/StepSkills"
import { StepCredentials } from "./steps/StepCredentials"
import type { ClientTemplate } from "@/components/client/ClientTemplates"
import type { ClientMode, SetClientModeParams, SetClientTemplateParams } from "@/types/tauri-commands"
import type { McpPermissions, SkillsPermissions, ModelPermissions } from "@/components/permissions/types"

type RoutingMode = "allowed" | "auto"

export interface AutoModelConfig {
  enabled: boolean
  model_name: string
  prioritized_models: [string, string][]
  available_models: [string, string][]
  routellm_config?: RouteLLMConfig
}

export interface RouteLLMConfig {
  enabled: boolean
  threshold: number
  weak_models: [string, string][]
}

// Logical step identifiers
type StepId = "welcome" | "template" | "mode" | "name" | "models" | "mcp" | "skills" | "credentials"

interface StepDef {
  id: StepId
  title: string
  description: string
}

interface WizardState {
  // Template & Mode
  selectedTemplate: ClientTemplate | null
  clientMode: ClientMode

  // Name
  clientName: string

  // Models
  routingMode: RoutingMode
  modelPermissions: ModelPermissions
  autoModelName: string
  prioritizedModels: [string, string][]
  routeLLMEnabled: boolean
  routeLLMThreshold: number
  weakModels: [string, string][]

  // MCP
  mcpPermissions: McpPermissions

  // Skills
  skillsPermissions: SkillsPermissions

  // After creation
  clientId?: string
  clientUuid?: string
  clientSecret?: string
}

interface ClientCreationWizardProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onComplete: (clientId: string) => void
  /** Show welcome step on first launch */
  showWelcome?: boolean
}

interface ClientInfo {
  id: string
  client_id: string
  strategy_id: string
  name: string
}

const INITIAL_STATE: WizardState = {
  selectedTemplate: null,
  clientMode: "both",
  clientName: "",
  routingMode: "allowed",
  modelPermissions: {
    global: "allow",
    providers: {},
    models: {},
  },
  autoModelName: "localrouter/auto",
  prioritizedModels: [],
  routeLLMEnabled: false,
  routeLLMThreshold: 0.3,
  weakModels: [],
  mcpPermissions: {
    global: "off",
    servers: {},
    tools: {},
    resources: {},
    prompts: {},
  },
  skillsPermissions: {
    global: "off",
    skills: {},
    tools: {},
  },
}

export function ClientCreationWizard({
  open,
  onOpenChange,
  onComplete,
  showWelcome = false,
}: ClientCreationWizardProps) {
  const [currentStep, setCurrentStep] = useState(0)
  const [creating, setCreating] = useState(false)
  const [state, setState] = useState<WizardState>(INITIAL_STATE)

  // Build visible steps dynamically based on state
  const visibleSteps = useMemo<StepDef[]>(() => {
    const steps: StepDef[] = []

    if (showWelcome) {
      steps.push({ id: "welcome", title: "Welcome", description: "Get started with LocalRouter." })
    }

    steps.push({ id: "template", title: "Choose Application", description: "Select an app to connect to LocalRouter." })

    // Mode step: only shown for non-custom templates
    const isCustom = !state.selectedTemplate || state.selectedTemplate.id === "custom"
    if (!isCustom) {
      steps.push({ id: "mode", title: "Client Mode", description: "Choose what this client can access." })
    }

    steps.push({ id: "name", title: "Name Your Client", description: "Choose a descriptive name for your client." })

    // Models step: skipped for mcp_only
    if (state.clientMode !== "mcp_only") {
      steps.push({ id: "models", title: "Select Models", description: "Choose which models this client can access." })
    }

    // MCP step: skipped for llm_only
    if (state.clientMode !== "llm_only") {
      steps.push({ id: "mcp", title: "Select MCP Servers", description: "Optionally configure MCP server access." })
    }

    // Skills step: skipped for llm_only
    if (state.clientMode !== "llm_only") {
      steps.push({ id: "skills", title: "Select Skills", description: "Optionally configure skills access." })
    }

    steps.push({ id: "credentials", title: "Your Credentials", description: "Save your credentials securely." })

    return steps
  }, [showWelcome, state.selectedTemplate, state.clientMode])

  const currentStepDef = visibleSteps[currentStep]
  const isFirstStep = currentStep === 0
  const isLastStep = currentStep === visibleSteps.length - 1
  const isCredentialsStep = currentStepDef?.id === "credentials"
  const isTemplateStep = currentStepDef?.id === "template"

  // Find the step that precedes credentials (the "create" step)
  const preCredentialsStepIndex = visibleSteps.findIndex(s => s.id === "credentials") - 1
  const isPreCredentialsStep = currentStep === preCredentialsStepIndex

  const canProceed = () => {
    if (!currentStepDef) return false
    switch (currentStepDef.id) {
      case "welcome": return true
      case "template": return false // Template step auto-advances on selection
      case "mode": return true
      case "name": return state.clientName.trim().length > 0
      case "models": return true
      case "mcp": return true
      case "skills": return true
      case "credentials": return true
      default: return false
    }
  }

  const handleNext = async () => {
    if (isPreCredentialsStep) {
      await createClient()
    } else if (!isLastStep) {
      setCurrentStep((prev) => prev + 1)
    }
  }

  const handleBack = () => {
    if (!isFirstStep && !isCredentialsStep) {
      setCurrentStep((prev) => prev - 1)
    }
  }

  const handleTemplateSelect = (template: ClientTemplate) => {
    const isCustom = template.id === "custom"
    setState((prev) => ({
      ...prev,
      selectedTemplate: template,
      clientMode: isCustom ? "both" : template.defaultMode,
      clientName: prev.clientName || (isCustom ? "" : template.name),
    }))
    // Auto-advance to next step
    setCurrentStep((prev) => prev + 1)
  }

  const createClient = async () => {
    try {
      setCreating(true)

      // Step 1: Create the client
      const [secret, clientInfo] = await invoke<[string, ClientInfo]>("create_client", {
        name: state.clientName.trim(),
      })

      // Step 2: Set client mode and template
      if (state.clientMode !== "both") {
        await invoke("set_client_mode", {
          clientId: clientInfo.client_id,
          mode: state.clientMode,
        } satisfies SetClientModeParams)
      }

      if (state.selectedTemplate && state.selectedTemplate.id !== "custom") {
        await invoke("set_client_template", {
          clientId: clientInfo.client_id,
          templateId: state.selectedTemplate.id,
        } satisfies SetClientTemplateParams)
      }

      // Step 3: Update strategy with auto routing configuration (if auto mode)
      if (state.routingMode === "auto") {
        const autoConfig: AutoModelConfig = {
          enabled: true,
          model_name: state.autoModelName,
          prioritized_models: state.prioritizedModels,
          available_models: [],
          routellm_config: state.routeLLMEnabled ? {
            enabled: true,
            threshold: state.routeLLMThreshold,
            weak_models: state.weakModels,
          } : undefined,
        }

        await invoke("update_strategy", {
          strategyId: clientInfo.strategy_id,
          allowedModels: null,
          autoConfig,
        })
      }

      // Step 4: Set model permissions using the new hierarchical system
      await invoke("set_client_model_permission", {
        clientId: clientInfo.client_id,
        level: "global",
        key: null,
        state: state.modelPermissions.global,
      })
      for (const [provider, permState] of Object.entries(state.modelPermissions.providers)) {
        await invoke("set_client_model_permission", {
          clientId: clientInfo.client_id,
          level: "provider",
          key: provider,
          state: permState,
        })
      }
      for (const [key, permState] of Object.entries(state.modelPermissions.models)) {
        await invoke("set_client_model_permission", {
          clientId: clientInfo.client_id,
          level: "model",
          key,
          state: permState,
        })
      }

      // Step 5: Set MCP permissions
      await invoke("set_client_mcp_permission", {
        clientId: clientInfo.client_id,
        level: "global",
        key: null,
        state: state.mcpPermissions.global,
      })
      for (const [serverId, permState] of Object.entries(state.mcpPermissions.servers)) {
        await invoke("set_client_mcp_permission", {
          clientId: clientInfo.client_id,
          level: "server",
          key: serverId,
          state: permState,
        })
      }
      for (const [key, permState] of Object.entries(state.mcpPermissions.tools)) {
        await invoke("set_client_mcp_permission", {
          clientId: clientInfo.client_id,
          level: "tool",
          key,
          state: permState,
        })
      }
      for (const [key, permState] of Object.entries(state.mcpPermissions.resources)) {
        await invoke("set_client_mcp_permission", {
          clientId: clientInfo.client_id,
          level: "resource",
          key,
          state: permState,
        })
      }
      for (const [key, permState] of Object.entries(state.mcpPermissions.prompts)) {
        await invoke("set_client_mcp_permission", {
          clientId: clientInfo.client_id,
          level: "prompt",
          key,
          state: permState,
        })
      }

      // Step 6: Set skills permissions
      await invoke("set_client_skills_permission", {
        clientId: clientInfo.client_id,
        level: "global",
        key: null,
        state: state.skillsPermissions.global,
      })
      for (const [skillName, permState] of Object.entries(state.skillsPermissions.skills)) {
        await invoke("set_client_skills_permission", {
          clientId: clientInfo.client_id,
          level: "skill",
          key: skillName,
          state: permState,
        })
      }
      for (const [key, permState] of Object.entries(state.skillsPermissions.tools)) {
        await invoke("set_client_skills_permission", {
          clientId: clientInfo.client_id,
          level: "tool",
          key,
          state: permState,
        })
      }

      // Update state with created client info
      setState((prev) => ({
        ...prev,
        clientId: clientInfo.client_id,
        clientUuid: clientInfo.id,
        clientSecret: secret,
      }))

      toast.success("Client created successfully")
      // Jump to credentials step
      const credIdx = visibleSteps.findIndex(s => s.id === "credentials")
      setCurrentStep(credIdx)
    } catch (error) {
      console.error("Failed to create client:", error)
      toast.error(`Failed to create client: ${error}`)
    } finally {
      setCreating(false)
    }
  }

  const handleComplete = () => {
    if (state.clientUuid) {
      onComplete(state.clientUuid)
    }
    handleClose()
  }

  const handleClose = () => {
    setCurrentStep(0)
    setState(INITIAL_STATE)
    onOpenChange(false)
  }

  const renderStep = () => {
    if (!currentStepDef) return null

    switch (currentStepDef.id) {
      case "welcome":
        return <StepWelcome />
      case "template":
        return <StepTemplate onSelect={handleTemplateSelect} />
      case "mode":
        return (
          <StepMode
            mode={state.clientMode}
            onChange={(mode) => setState((prev) => ({ ...prev, clientMode: mode }))}
            template={state.selectedTemplate}
          />
        )
      case "name":
        return (
          <StepName
            name={state.clientName}
            onChange={(name) => setState((prev) => ({ ...prev, clientName: name }))}
          />
        )
      case "models":
        return (
          <StepModels
            routingMode={state.routingMode}
            modelPermissions={state.modelPermissions}
            autoModelName={state.autoModelName}
            prioritizedModels={state.prioritizedModels}
            routeLLMEnabled={state.routeLLMEnabled}
            routeLLMThreshold={state.routeLLMThreshold}
            weakModels={state.weakModels}
            onRoutingModeChange={(mode) =>
              setState((prev) => ({ ...prev, routingMode: mode }))
            }
            onModelPermissionsChange={(permissions) =>
              setState((prev) => ({ ...prev, modelPermissions: permissions }))
            }
            onAutoModelNameChange={(name) =>
              setState((prev) => ({ ...prev, autoModelName: name }))
            }
            onPrioritizedModelsChange={(models) =>
              setState((prev) => ({ ...prev, prioritizedModels: models }))
            }
            onRouteLLMEnabledChange={(enabled) =>
              setState((prev) => ({ ...prev, routeLLMEnabled: enabled }))
            }
            onRouteLLMThresholdChange={(threshold) =>
              setState((prev) => ({ ...prev, routeLLMThreshold: threshold }))
            }
            onWeakModelsChange={(models) =>
              setState((prev) => ({ ...prev, weakModels: models }))
            }
          />
        )
      case "mcp":
        return (
          <StepMcp
            permissions={state.mcpPermissions}
            onChange={(permissions) =>
              setState((prev) => ({ ...prev, mcpPermissions: permissions }))
            }
          />
        )
      case "skills":
        return (
          <StepSkills
            permissions={state.skillsPermissions}
            onChange={(permissions) =>
              setState((prev) => ({ ...prev, skillsPermissions: permissions }))
            }
          />
        )
      case "credentials":
        return (
          <StepCredentials
            clientId={state.clientId || ""}
            clientUuid={state.clientUuid || ""}
            secret={state.clientSecret || null}
            templateId={state.selectedTemplate?.id !== "custom" ? state.selectedTemplate?.id : null}
            clientMode={state.clientMode}
          />
        )
      default:
        return null
    }
  }

  // For template step, don't show Next button (auto-advances on selection)
  const showNextButton = !isTemplateStep

  return (
    <Dialog open={open} onOpenChange={isCredentialsStep ? undefined : handleClose}>
      <DialogContent className="sm:max-w-[600px]">
        <DialogHeader>
          <div className="flex items-center gap-2 mb-1">
            <span className="text-xs font-medium text-muted-foreground">
              Step {currentStep + 1} of {visibleSteps.length}
            </span>
            <div className="flex-1 h-1 bg-muted rounded-full overflow-hidden">
              <div
                className="h-full bg-primary transition-all duration-300"
                style={{ width: `${((currentStep + 1) / visibleSteps.length) * 100}%` }}
              />
            </div>
          </div>
          <DialogTitle>{currentStepDef?.title}</DialogTitle>
          <DialogDescription>{currentStepDef?.description}</DialogDescription>
        </DialogHeader>

        <div className="py-4 px-1 min-h-[300px] max-h-[60vh] overflow-y-auto">{renderStep()}</div>

        <DialogFooter className="flex justify-between sm:justify-between">
          <div>
            {!isFirstStep && !isCredentialsStep && (
              <Button variant="outline" onClick={handleBack} disabled={creating}>
                <ChevronLeft className="mr-1 h-4 w-4" />
                Back
              </Button>
            )}
          </div>
          <div className="flex gap-2">
            {isCredentialsStep ? (
              <Button onClick={handleComplete}>Done</Button>
            ) : showNextButton ? (
              <Button onClick={handleNext} disabled={!canProceed() || creating}>
                {creating ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    Creating...
                  </>
                ) : isPreCredentialsStep ? (
                  "Create Client"
                ) : (
                  <>
                    Next
                    <ChevronRight className="ml-1 h-4 w-4" />
                  </>
                )}
              </Button>
            ) : null}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
