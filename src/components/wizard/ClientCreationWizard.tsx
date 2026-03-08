/**
 * ClientCreationWizard
 *
 * Streamlined 4-step wizard for creating a new client:
 * 1. Template - Select an app template (Claude Code, Cursor, etc.)
 * 2. Name + Mode - Name the client and select LLM / MCP / Both
 * 3. Models - Configure model access using StrategyModelConfiguration (skipped for mcp_only)
 * 4. Credentials - View and copy the generated credentials
 *
 * The client is created after step 2 (Name + Mode) so that step 3 (Models)
 * can use the real StrategyModelConfiguration component which saves directly
 * to the backend - fixing the auto router bug and staying in sync with the
 * rest of the app.
 */

import { useState, useMemo, useEffect, useCallback } from "react"
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
import { StepTemplate } from "./steps/StepTemplate"
import { StepNameAndMode } from "./steps/StepNameAndMode"
import { StepCredentials } from "./steps/StepCredentials"
import { StrategyModelConfiguration } from "@/components/strategy/StrategyModelConfiguration"
import { CLIENT_TEMPLATES, type ClientTemplate } from "@/components/client/ClientTemplates"
import type { ClientMode, SetClientModeParams, SetClientTemplateParams } from "@/types/tauri-commands"
import type { ModelPermissions } from "@/components/permissions/types"

// Logical step identifiers
type StepId = "template" | "name_mode" | "models" | "credentials"

interface StepDef {
  id: StepId
  title: string
  description: string
}

interface WizardState {
  // Template & Mode
  selectedTemplate: ClientTemplate | null
  clientMode: ClientMode
  clientName: string

  // After creation (set after step 2)
  clientId?: string
  clientUuid?: string
  clientSecret?: string
  strategyId?: string
  modelPermissions?: ModelPermissions
}

interface ClientCreationWizardProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onComplete: (clientId: string) => void
  /** Pre-select a template by ID and skip the template step */
  initialTemplateId?: string | null
}

interface ClientInfo {
  id: string
  client_id: string
  strategy_id: string
  name: string
  model_permissions: ModelPermissions
}

const INITIAL_STATE: WizardState = {
  selectedTemplate: null,
  clientMode: "both",
  clientName: "",
}

export function ClientCreationWizard({
  open,
  onOpenChange,
  onComplete,
  initialTemplateId,
}: ClientCreationWizardProps) {
  const [currentStep, setCurrentStep] = useState(0)
  const [creating, setCreating] = useState(false)
  const [state, setState] = useState<WizardState>(INITIAL_STATE)

  // Pre-select template when wizard opens with initialTemplateId
  useEffect(() => {
    if (open && initialTemplateId) {
      const template = CLIENT_TEMPLATES.find(t => t.id === initialTemplateId)
      if (template) {
        const isCustom = template.id === "custom"
        setState(prev => ({
          ...prev,
          selectedTemplate: template,
          clientMode: isCustom ? "both" : template.defaultMode,
          clientName: prev.clientName || (isCustom ? "" : template.name),
        }))
        // Skip to name+mode step
        setCurrentStep(1)
      }
    }
  }, [open, initialTemplateId])

  // Build visible steps dynamically based on state
  const visibleSteps = useMemo<StepDef[]>(() => {
    const steps: StepDef[] = [
      { id: "template", title: "Choose Application", description: "Select an app to connect to LocalRouter." },
      { id: "name_mode", title: "Setup", description: "Name your client and choose what it can access." },
    ]

    // Models step: skipped for mcp_only
    if (state.clientMode !== "mcp_only") {
      steps.push({ id: "models", title: "Configure Models", description: "Choose how models are selected and accessed." })
    }

    steps.push({ id: "credentials", title: "Your Credentials", description: "Save your credentials securely." })

    return steps
  }, [state.clientMode])

  const currentStepDef = visibleSteps[currentStep]
  const isFirstStep = currentStep === 0
  const isLastStep = currentStep === visibleSteps.length - 1
  const isCredentialsStep = currentStepDef?.id === "credentials"
  const isTemplateStep = currentStepDef?.id === "template"
  const isNameModeStep = currentStepDef?.id === "name_mode"
  const isModelsStep = currentStepDef?.id === "models"

  const canProceed = () => {
    if (!currentStepDef) return false
    switch (currentStepDef.id) {
      case "template": return false // Auto-advances on selection
      case "name_mode": return state.clientName.trim().length > 0
      case "models": return true
      case "credentials": return true
      default: return false
    }
  }

  const handleNext = async () => {
    if (isNameModeStep) {
      // Create client after name+mode step, before models
      await createClient()
    } else if (isModelsStep) {
      // Models step saves directly to backend via StrategyModelConfiguration,
      // just advance to credentials
      const credIdx = visibleSteps.findIndex(s => s.id === "credentials")
      setCurrentStep(credIdx)
    } else if (!isLastStep) {
      setCurrentStep((prev) => prev + 1)
    }
  }

  const handleBack = () => {
    if (!isFirstStep && !isCredentialsStep) {
      // If going back from models step, we already created the client.
      // That's fine - user can still modify settings, and re-advancing
      // won't re-create (we check for existing clientId).
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
    // Skip if client already created (user went back and forward again)
    if (state.clientId) {
      const modelsIdx = visibleSteps.findIndex(s => s.id === "models")
      const credIdx = visibleSteps.findIndex(s => s.id === "credentials")
      setCurrentStep(modelsIdx !== -1 ? modelsIdx : credIdx)
      return
    }

    try {
      setCreating(true)

      // Step 1: Create the client
      const [secret, clientInfo] = await invoke<[string, ClientInfo]>("create_client", {
        name: state.clientName.trim(),
      })

      // Step 2: Set client mode
      if (state.clientMode !== "both") {
        await invoke("set_client_mode", {
          clientId: clientInfo.client_id,
          mode: state.clientMode,
        } satisfies SetClientModeParams)
      }

      // Step 3: Set template
      if (state.selectedTemplate && state.selectedTemplate.id !== "custom") {
        await invoke("set_client_template", {
          clientId: clientInfo.client_id,
          templateId: state.selectedTemplate.id,
        } satisfies SetClientTemplateParams)
      }

      // Update state with created client info
      setState((prev) => ({
        ...prev,
        clientId: clientInfo.client_id,
        clientUuid: clientInfo.id,
        clientSecret: secret,
        strategyId: clientInfo.strategy_id,
        modelPermissions: clientInfo.model_permissions,
      }))

      // Advance to models step or credentials (if mcp_only)
      const modelsIdx = visibleSteps.findIndex(s => s.id === "models")
      const credIdx = visibleSteps.findIndex(s => s.id === "credentials")
      setCurrentStep(modelsIdx !== -1 ? modelsIdx : credIdx)
    } catch (error) {
      console.error("Failed to create client:", error)
      toast.error(`Failed to create client: ${error}`)
    } finally {
      setCreating(false)
    }
  }

  const handleClientUpdate = useCallback(() => {
    // Reload model permissions after StrategyModelConfiguration makes changes
    if (state.clientUuid) {
      invoke<ClientInfo[]>("list_clients")
        .then((clients) => {
          const client = clients.find(c => c.id === state.clientUuid)
          if (client) {
            setState((prev) => ({
              ...prev,
              modelPermissions: client.model_permissions,
            }))
          }
        })
        .catch(() => {
          // Non-critical, permissions will be correct on next load
        })
    }
  }, [state.clientUuid])

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
      case "template":
        return <StepTemplate onSelect={handleTemplateSelect} />
      case "name_mode":
        return (
          <StepNameAndMode
            name={state.clientName}
            onNameChange={(name) => setState((prev) => ({ ...prev, clientName: name }))}
            mode={state.clientMode}
            onModeChange={(mode) => setState((prev) => ({ ...prev, clientMode: mode }))}
            template={state.selectedTemplate}
          />
        )
      case "models":
        if (!state.strategyId || !state.clientId) return null
        return (
          <div className="wizard-compact-cards [&>div>div]:ml-0">
            <StrategyModelConfiguration
              strategyId={state.strategyId}
              clientContext={state.modelPermissions ? {
                clientId: state.clientId,
                modelPermissions: state.modelPermissions,
                onClientUpdate: handleClientUpdate,
              } : undefined}
            />
          </div>
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
                ) : isNameModeStep ? (
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
