/**
 * ClientCreationWizard
 *
 * Multi-step wizard for creating a new client with guided setup.
 * Steps (when showWelcome is true, adds Welcome step at the beginning):
 * 0. Welcome - Introduction to LocalRouter (first launch only)
 * 1. Name - Choose a name for the client
 * 2. Models - Select which models the client can access
 * 3. MCP - Select which MCP servers the client can access (optional)
 * 4. Skills - Select which skills the client can access (optional)
 * 5. Credentials - View and copy the generated credentials
 */

import { useState } from "react"
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
import { StepName } from "./steps/StepName"
import { StepModels } from "./steps/StepModels"
import { StepMcp } from "./steps/StepMcp"
import { StepSkills } from "./steps/StepSkills"
import { StepCredentials } from "./steps/StepCredentials"
import type { AllowedModelsSelection } from "@/components/strategy/AllowedModelsSelector"

type McpAccessMode = "none" | "all" | "specific"
type SkillsAccessMode = "none" | "all" | "specific"
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

interface WizardState {
  // Step 1
  clientName: string

  // Step 2 - Models
  routingMode: RoutingMode
  allowedModels: AllowedModelsSelection
  autoModelName: string
  prioritizedModels: [string, string][]
  routeLLMEnabled: boolean
  routeLLMThreshold: number
  weakModels: [string, string][]

  // Step 3 - MCP
  mcpAccessMode: McpAccessMode
  selectedMcpServers: string[]

  // Step 4 - Skills
  skillsAccessMode: SkillsAccessMode
  selectedSkillPaths: string[]

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

const BASE_STEP_TITLES = [
  "Name Your Client",
  "Select Models",
  "Select MCP Servers",
  "Select Skills",
  "Your Credentials",
]

const BASE_STEP_DESCRIPTIONS = [
  "Choose a descriptive name for your client.",
  "Choose which models this client can access.",
  "Optionally configure MCP server access.",
  "Optionally configure skills access.",
  "Save your credentials securely.",
]

const WELCOME_STEP_TITLE = "Welcome"
const WELCOME_STEP_DESCRIPTION = "Get started with LocalRouter."

export function ClientCreationWizard({
  open,
  onOpenChange,
  onComplete,
  showWelcome = false,
}: ClientCreationWizardProps) {
  const [currentStep, setCurrentStep] = useState(0)
  const [creating, setCreating] = useState(false)
  const [state, setState] = useState<WizardState>({
    clientName: "",
    routingMode: "allowed",
    allowedModels: {
      selected_all: true,
      selected_providers: [],
      selected_models: [],
    },
    autoModelName: "localrouter/auto",
    prioritizedModels: [],
    routeLLMEnabled: false,
    routeLLMThreshold: 0.3,
    weakModels: [],
    mcpAccessMode: "none",
    selectedMcpServers: [],
    skillsAccessMode: "none",
    selectedSkillPaths: [],
  })

  // Build step arrays based on whether welcome is shown
  const stepTitles = showWelcome
    ? [WELCOME_STEP_TITLE, ...BASE_STEP_TITLES]
    : BASE_STEP_TITLES
  const stepDescriptions = showWelcome
    ? [WELCOME_STEP_DESCRIPTION, ...BASE_STEP_DESCRIPTIONS]
    : BASE_STEP_DESCRIPTIONS

  // Calculate step indices (offset by 1 if welcome is shown)
  const offset = showWelcome ? 1 : 0
  const nameStepIndex = 0 + offset
  const modelsStepIndex = 1 + offset
  const mcpStepIndex = 2 + offset
  const skillsStepIndex = 3 + offset
  const credentialsStepIndex = 4 + offset

  const isFirstStep = currentStep === 0
  const isLastStep = currentStep === stepTitles.length - 1
  const isCredentialsStep = currentStep === credentialsStepIndex

  const canProceed = () => {
    // Welcome step can always proceed
    if (showWelcome && currentStep === 0) {
      return true
    }
    // Map current step to the logical step for validation
    if (currentStep === nameStepIndex) {
      return state.clientName.trim().length > 0
    }
    if (currentStep === modelsStepIndex) {
      return true // Models always valid (default is all)
    }
    if (currentStep === mcpStepIndex) {
      return true // MCP is optional
    }
    if (currentStep === skillsStepIndex) {
      return true // Skills are optional
    }
    if (currentStep === credentialsStepIndex) {
      return true // Can always close from credentials
    }
    return false
  }

  const handleNext = async () => {
    if (currentStep === skillsStepIndex) {
      // Create client before moving to credentials step
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

  const createClient = async () => {
    try {
      setCreating(true)

      // Step 1: Create the client
      const [secret, clientInfo] = await invoke<[string, ClientInfo]>("create_client", {
        name: state.clientName.trim(),
      })

      // Step 2: Update strategy with model routing configuration
      const autoConfig: AutoModelConfig | null = state.routingMode === "auto" ? {
        enabled: true,
        model_name: state.autoModelName,
        prioritized_models: state.prioritizedModels,
        available_models: [],
        routellm_config: state.routeLLMEnabled ? {
          enabled: true,
          threshold: state.routeLLMThreshold,
          weak_models: state.weakModels,
        } : undefined,
      } : null

      await invoke("update_strategy", {
        strategyId: clientInfo.strategy_id,
        allowedModels: state.routingMode === "allowed" ? state.allowedModels : null,
        autoConfig,
      })

      // Step 3: Set MCP access
      await invoke("set_client_mcp_access", {
        clientId: clientInfo.client_id,
        mode: state.mcpAccessMode,
        servers: state.selectedMcpServers,
      })

      // Step 4: Set skills access
      if (state.skillsAccessMode !== "none") {
        await invoke("set_client_skills_access", {
          clientId: clientInfo.client_id,
          mode: state.skillsAccessMode,
          paths: state.selectedSkillPaths,
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
      setCurrentStep(credentialsStepIndex)
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
    // Reset state when closing
    setCurrentStep(0)
    setState({
      clientName: "",
      routingMode: "allowed",
      allowedModels: {
        selected_all: true,
        selected_providers: [],
        selected_models: [],
      },
      autoModelName: "localrouter/auto",
      prioritizedModels: [],
      routeLLMEnabled: false,
      routeLLMThreshold: 0.3,
      weakModels: [],
      mcpAccessMode: "none",
      selectedMcpServers: [],
      skillsAccessMode: "none",
      selectedSkillPaths: [],
    })
    onOpenChange(false)
  }

  const renderStep = () => {
    // Welcome step (only when showWelcome is true and on step 0)
    if (showWelcome && currentStep === 0) {
      return <StepWelcome />
    }

    if (currentStep === nameStepIndex) {
      return (
        <StepName
          name={state.clientName}
          onChange={(name) => setState((prev) => ({ ...prev, clientName: name }))}
        />
      )
    }
    if (currentStep === modelsStepIndex) {
      return (
        <StepModels
          routingMode={state.routingMode}
          allowedModels={state.allowedModels}
          autoModelName={state.autoModelName}
          prioritizedModels={state.prioritizedModels}
          routeLLMEnabled={state.routeLLMEnabled}
          routeLLMThreshold={state.routeLLMThreshold}
          weakModels={state.weakModels}
          onRoutingModeChange={(mode) =>
            setState((prev) => ({ ...prev, routingMode: mode }))
          }
          onAllowedModelsChange={(selection) =>
            setState((prev) => ({ ...prev, allowedModels: selection }))
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
    }
    if (currentStep === mcpStepIndex) {
      return (
        <StepMcp
          accessMode={state.mcpAccessMode}
          selectedServers={state.selectedMcpServers}
          onChange={(mode, servers) =>
            setState((prev) => ({
              ...prev,
              mcpAccessMode: mode,
              selectedMcpServers: servers,
            }))
          }
        />
      )
    }
    if (currentStep === skillsStepIndex) {
      return (
        <StepSkills
          accessMode={state.skillsAccessMode}
          selectedPaths={state.selectedSkillPaths}
          onChange={(mode, paths) =>
            setState((prev) => ({
              ...prev,
              skillsAccessMode: mode,
              selectedSkillPaths: paths,
            }))
          }
        />
      )
    }
    if (currentStep === credentialsStepIndex) {
      return (
        <StepCredentials
          clientId={state.clientId || ""}
          clientUuid={state.clientUuid || ""}
          secret={state.clientSecret || null}
        />
      )
    }
    return null
  }

  return (
    <Dialog open={open} onOpenChange={isCredentialsStep ? undefined : handleClose}>
      <DialogContent className="sm:max-w-[600px]">
        <DialogHeader>
          <div className="flex items-center gap-2 mb-1">
            <span className="text-xs font-medium text-muted-foreground">
              Step {currentStep + 1} of {stepTitles.length}
            </span>
            <div className="flex-1 h-1 bg-muted rounded-full overflow-hidden">
              <div
                className="h-full bg-primary transition-all duration-300"
                style={{ width: `${((currentStep + 1) / stepTitles.length) * 100}%` }}
              />
            </div>
          </div>
          <DialogTitle>{stepTitles[currentStep]}</DialogTitle>
          <DialogDescription>{stepDescriptions[currentStep]}</DialogDescription>
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
            ) : (
              <Button onClick={handleNext} disabled={!canProceed() || creating}>
                {creating ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    Creating...
                  </>
                ) : currentStep === skillsStepIndex ? (
                  "Create Client"
                ) : (
                  <>
                    Next
                    <ChevronRight className="ml-1 h-4 w-4" />
                  </>
                )}
              </Button>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
