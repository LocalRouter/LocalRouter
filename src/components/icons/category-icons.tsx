/**
 * Standardized category icons for the three main resource types.
 *
 * Change an icon here and it updates everywhere in the app:
 * sidebar, command palette, list views, empty states, etc.
 *
 * Icons sourced from Google Material Symbols and Lucide.
 */
import type { SVGProps } from "react"

/** LLM Providers — "network_intelligence" icon from Google Material Symbols */
export function ProvidersIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 -960 960 960"
      fill="currentColor"
      {...props}
    >
      <path d="M323-160q-11 0-20.5-5.5T288-181l-78-139h58l40 80h92v-40h-68l-40-80H188l-57-100q-2-5-3.5-10t-1.5-10q0-4 5-20l57-100h104l40-80h68v-40h-92l-40 80h-58l78-139q5-10 14.5-15.5T323-800h97q17 0 28.5 11.5T460-760v160h-60l-40 40h100v120h-88l-40-80h-92l-40 40h108l40 80h112v200q0 17-11.5 28.5T420-160h-97Zm217 0q-17 0-28.5-11.5T500-200v-200h112l40-80h108l-40-40h-92l-40 80h-88v-120h100l-40-40h-60v-160q0-17 11.5-28.5T540-800h97q11 0 20.5 5.5T672-779l78 139h-58l-40-80h-92v40h68l40 80h104l57 100q2 5 3.5 10t1.5 10q0 4-5 20l-57 100H668l-40 80h-68v40h92l40-80h58l-78 139q-5 10-14.5 15.5T637-160h-97Z" />
    </svg>
  )
}

/** MCP Servers — "server" icon from Lucide */
export function McpIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      {...props}
    >
      <rect width="20" height="8" x="2" y="2" rx="2" ry="2" />
      <rect width="20" height="8" x="2" y="14" rx="2" ry="2" />
      <line x1="6" x2="6.01" y1="6" y2="6" />
      <line x1="6" x2="6.01" y1="18" y2="18" />
    </svg>
  )
}

/** Skills — "tools_power_drill" icon from Google Material Symbols */
export function SkillsIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 -960 960 960"
      fill="currentColor"
      {...props}
    >
      <path d="M240-200h240v-40H240v40Zm30-360h180q12 0 21-9t9-21q0-12-9-21t-21-9H270q-12 0-21 9t-9 21q0 12 9 21t21 9Zm0-100h180q12 0 21-9t9-21q0-12-9-21t-21-9H270q-12 0-21 9t-9 21q0 12 9 21t21 9Zm370 140v-80h80v-80h-80v-80h80q33 0 56.5 23.5T800-680h80q17 0 28.5 11.5T920-640q0 17-11.5 28.5T880-600h-80q0 33-23.5 56.5T720-520h-80ZM480-320h-80v-200h160v-240H240q-33 0-56.5 23.5T160-680v80q0 33 23.5 56.5T240-520h80v200h-80v-120q-66 0-113-47T80-600v-80q0-66 47-113t113-47h320q33 0 56.5 23.5T640-760v240q0 33-23.5 56.5T560-440h-80v120ZM220-120q-25 0-42.5-17.5T160-180v-80q0-25 17.5-42.5T220-320h280q25 0 42.5 17.5T560-260v80q0 25-17.5 42.5T500-120H220Zm140-520Zm120 440H240h240Z" />
    </svg>
  )
}

/** Store / Marketplace — "storefront" icon from Google Material Symbols */
export function StoreIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 -960 960 960"
      fill="currentColor"
      {...props}
    >
      <path d="M160-720v-80h640v80H160Zm0 560v-240h-40v-80l40-200h640l40 200v80h-40v240h-80v-240H560v240H160Zm80-80h240v-160H240v160Zm-38-240h556-556Zm0 0h556l-24-120H226l-24 120Z" />
    </svg>
  )
}
