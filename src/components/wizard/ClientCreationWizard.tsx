/**
 * ClientCreationWizard
 *
 * Streamlined wizard for creating a new client:
 * 0. Welcome - Introduction to LocalRouter (first launch only, optional)
 * 1. Template - Select an app template (Claude Code, Cursor, etc.)
 * 2. Name + Mode - Name the client and select LLM / MCP / Both (creates client)
 */

import { useState, useMemo, useEffect, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { ChevronLeft, Loader2 } from "lucide-react"
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
import { StepNameAndMode } from "./steps/StepNameAndMode"
import { CLIENT_TEMPLATES, type ClientTemplate } from "@/components/client/ClientTemplates"
import type { ClientMode, SetClientModeParams, SetClientTemplateParams } from "@/types/tauri-commands"

// Logical step identifiers
type StepId = "welcome" | "template" | "name_mode"

interface StepDef {
  id: StepId
  title: string
  description: string
}

interface WizardState {
  selectedTemplate: ClientTemplate | null
  clientMode: ClientMode
  clientName: string
}

interface ClientCreationWizardProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onComplete: (clientId: string) => void
  /** Show welcome step on first launch */
  showWelcome?: boolean
  /** Pre-select a template by ID and skip the template step */
  initialTemplateId?: string | null
}

interface ClientInfo {
  id: string
  client_id: string
  name: string
}

const INITIAL_STATE: WizardState = {
  selectedTemplate: null,
  clientMode: "both",
  clientName: "",
}

/** Generate a suggested name, appending (2), (3) etc. if the base name already exists */
function generateSuggestedName(baseName: string, existingNames: string[]): string {
  if (!existingNames.includes(baseName)) return baseName
  let n = 2
  while (true) {
    const candidate = `${baseName} (${n})`
    if (!existingNames.includes(candidate)) return candidate
    n++
  }
}

export function ClientCreationWizard({
  open,
  onOpenChange,
  onComplete,
  showWelcome = false,
  initialTemplateId,
}: ClientCreationWizardProps) {
  const [currentStep, setCurrentStep] = useState(0)
  const [creating, setCreating] = useState(false)
  const [state, setState] = useState<WizardState>(INITIAL_STATE)
  const existingNamesRef = useRef<string[]>([])

  // Load existing client names and pre-select template when wizard opens
  useEffect(() => {
    if (!open) return
    invoke<ClientInfo[]>("list_clients")
      .then((clients) => {
        existingNamesRef.current = clients.map(c => c.name)
      })
      .catch(() => {
        existingNamesRef.current = []
      })
      .finally(() => {
        if (initialTemplateId) {
          const template = CLIENT_TEMPLATES.find(t => t.id === initialTemplateId)
          if (template) {
            const isCustom = template.id === "custom"
            setState(prev => ({
              ...prev,
              selectedTemplate: template,
              clientMode: isCustom ? "both" : template.defaultMode,
              clientName: prev.clientName || (isCustom ? "" : generateSuggestedName(template.name, existingNamesRef.current)),
            }))
            setCurrentStep(showWelcome ? 2 : 1)
          }
        }
      })
  }, [open, initialTemplateId])

  // Build visible steps dynamically based on state
  const visibleSteps = useMemo<StepDef[]>(() => {
    const steps: StepDef[] = []

    if (showWelcome) {
      steps.push({ id: "welcome", title: "Welcome", description: "Get started with LocalRouter." })
    }

    steps.push(
      { id: "template", title: "Choose Application", description: "Select an app to connect to LocalRouter." },
      { id: "name_mode", title: "Setup", description: "Name your client and choose what it can access." },
    )

    return steps
  }, [showWelcome])

  const currentStepDef = visibleSteps[currentStep]
  const isFirstStep = currentStep === 0
  const isLastStep = currentStep === visibleSteps.length - 1
  const isTemplateStep = currentStepDef?.id === "template"
  const isNameModeStep = currentStepDef?.id === "name_mode"
  const canProceed = () => {
    if (!currentStepDef) return false
    switch (currentStepDef.id) {
      case "welcome": return true
      case "template": return false // Auto-advances on selection
      case "name_mode": return state.clientName.trim().length > 0
      default: return false
    }
  }

  const handleNext = async () => {
    if (isNameModeStep) {
      await createClient()
    } else if (!isLastStep) {
      setCurrentStep((prev) => prev + 1)
    }
  }

  const handleBack = () => {
    if (!isFirstStep) {
      setCurrentStep((prev) => prev - 1)
    }
  }

  const handleTemplateSelect = (template: ClientTemplate) => {
    const isCustom = template.id === "custom"
    setState((prev) => ({
      ...prev,
      selectedTemplate: template,
      clientMode: isCustom ? "both" : template.defaultMode,
      clientName: prev.clientName || (isCustom ? "" : generateSuggestedName(template.name, existingNamesRef.current)),
    }))
    // Auto-advance to next step
    setCurrentStep((prev) => prev + 1)
  }

  const createClient = async () => {
    try {
      setCreating(true)

      // Step 1: Create the client
      const [, clientInfo] = await invoke<[string, ClientInfo]>("create_client", {
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

      // Complete the wizard
      onComplete(clientInfo.id)
      handleClose()
    } catch (error) {
      console.error("Failed to create client:", error)
      toast.error(`Failed to create client: ${error}`)
    } finally {
      setCreating(false)
    }
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
      default:
        return null
    }
  }

  // For template step, don't show Next button (auto-advances on selection)
  const showNextButton = !isTemplateStep

  return (
    <Dialog open={open} onOpenChange={handleClose}>
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
            {!isFirstStep && (
              <Button variant="outline" onClick={handleBack} disabled={creating}>
                <ChevronLeft className="mr-1 h-4 w-4" />
                Back
              </Button>
            )}
          </div>
          <div className="flex gap-2">
            {showNextButton && (
              <Button onClick={handleNext} disabled={!canProceed() || creating}>
                {creating ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    Creating...
                  </>
                ) : (
                  "Create Client"
                )}
              </Button>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
