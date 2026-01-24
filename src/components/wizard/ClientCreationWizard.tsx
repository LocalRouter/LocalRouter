/**
 * ClientCreationWizard
 *
 * Multi-step wizard for creating a new client with guided setup.
 * Steps:
 * 1. Name - Choose a name for the client
 * 2. Models - Select which models the client can access
 * 3. MCP - Select which MCP servers the client can access (optional)
 * 4. Credentials - View and copy the generated credentials
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
import { StepName } from "./steps/StepName"
import { StepModels } from "./steps/StepModels"
import { StepMcp } from "./steps/StepMcp"
import { StepCredentials } from "./steps/StepCredentials"
import type { AllowedModelsSelection } from "@/components/strategy/AllowedModelsSelector"

type McpAccessMode = "none" | "all" | "specific"

interface WizardState {
  // Step 1
  clientName: string

  // Step 2 - Models
  allowedModels: AllowedModelsSelection

  // Step 3 - MCP
  mcpAccessMode: McpAccessMode
  selectedMcpServers: string[]

  // After creation
  clientId?: string
  clientSecret?: string
}

interface ClientCreationWizardProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onComplete: (clientId: string) => void
}

interface ClientInfo {
  id: string
  client_id: string
  strategy_id: string
  name: string
}

const STEP_TITLES = [
  "Name Your Client",
  "Select Models",
  "Select MCP Servers",
  "Your Credentials",
]

const STEP_DESCRIPTIONS = [
  "Choose a descriptive name for your client.",
  "Choose which models this client can access.",
  "Optionally configure MCP server access.",
  "Save your credentials securely.",
]

export function ClientCreationWizard({
  open,
  onOpenChange,
  onComplete,
}: ClientCreationWizardProps) {
  const [currentStep, setCurrentStep] = useState(0)
  const [creating, setCreating] = useState(false)
  const [state, setState] = useState<WizardState>({
    clientName: "",
    allowedModels: {
      selected_all: true,
      selected_providers: [],
      selected_models: [],
    },
    mcpAccessMode: "none",
    selectedMcpServers: [],
  })

  const isFirstStep = currentStep === 0
  const isLastStep = currentStep === STEP_TITLES.length - 1
  const isCredentialsStep = currentStep === 3

  const canProceed = () => {
    switch (currentStep) {
      case 0:
        return state.clientName.trim().length > 0
      case 1:
        return true // Models always valid (default is all)
      case 2:
        return true // MCP is optional
      case 3:
        return true // Can always close from credentials
      default:
        return false
    }
  }

  const handleNext = async () => {
    if (currentStep === 2) {
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

  const handleSkip = () => {
    if (currentStep === 2) {
      // Skip MCP and create client
      createClient()
    }
  }

  const createClient = async () => {
    try {
      setCreating(true)

      // Step 1: Create the client
      const [secret, clientInfo] = await invoke<[string, ClientInfo]>("create_client", {
        name: state.clientName.trim(),
      })

      // Step 2: Update strategy allowed models
      await invoke("update_strategy", {
        strategyId: clientInfo.strategy_id,
        allowedModels: state.allowedModels,
      })

      // Step 3: Set MCP access
      await invoke("set_client_mcp_access", {
        clientId: clientInfo.client_id,
        mode: state.mcpAccessMode,
        servers: state.selectedMcpServers,
      })

      // Update state with created client info
      setState((prev) => ({
        ...prev,
        clientId: clientInfo.id,
        clientSecret: secret,
      }))

      toast.success("Client created successfully")
      setCurrentStep(3)
    } catch (error) {
      console.error("Failed to create client:", error)
      toast.error(`Failed to create client: ${error}`)
    } finally {
      setCreating(false)
    }
  }

  const handleComplete = () => {
    if (state.clientId) {
      onComplete(state.clientId)
    }
    handleClose()
  }

  const handleClose = () => {
    // Reset state when closing
    setCurrentStep(0)
    setState({
      clientName: "",
      allowedModels: {
        selected_all: true,
        selected_providers: [],
        selected_models: [],
      },
      mcpAccessMode: "none",
      selectedMcpServers: [],
    })
    onOpenChange(false)
  }

  const renderStep = () => {
    switch (currentStep) {
      case 0:
        return (
          <StepName
            name={state.clientName}
            onChange={(name) => setState((prev) => ({ ...prev, clientName: name }))}
          />
        )
      case 1:
        return (
          <StepModels
            allowedModels={state.allowedModels}
            onChange={(selection) =>
              setState((prev) => ({ ...prev, allowedModels: selection }))
            }
          />
        )
      case 2:
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
      case 3:
        return (
          <StepCredentials
            clientId={state.clientId || ""}
            secret={state.clientSecret || null}
          />
        )
      default:
        return null
    }
  }

  return (
    <Dialog open={open} onOpenChange={isCredentialsStep ? undefined : handleClose}>
      <DialogContent className="sm:max-w-[600px]">
        <DialogHeader>
          <div className="flex items-center gap-2 mb-1">
            <span className="text-xs font-medium text-muted-foreground">
              Step {currentStep + 1} of {STEP_TITLES.length}
            </span>
            <div className="flex-1 h-1 bg-muted rounded-full overflow-hidden">
              <div
                className="h-full bg-primary transition-all duration-300"
                style={{ width: `${((currentStep + 1) / STEP_TITLES.length) * 100}%` }}
              />
            </div>
          </div>
          <DialogTitle>{STEP_TITLES[currentStep]}</DialogTitle>
          <DialogDescription>{STEP_DESCRIPTIONS[currentStep]}</DialogDescription>
        </DialogHeader>

        <div className="py-4 min-h-[300px]">{renderStep()}</div>

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
            {currentStep === 2 && state.mcpAccessMode === "none" && (
              <Button variant="ghost" onClick={handleSkip} disabled={creating}>
                Skip
              </Button>
            )}
            {isCredentialsStep ? (
              <Button onClick={handleComplete}>Done</Button>
            ) : (
              <Button onClick={handleNext} disabled={!canProceed() || creating}>
                {creating ? (
                  <>
                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    Creating...
                  </>
                ) : currentStep === 2 ? (
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
