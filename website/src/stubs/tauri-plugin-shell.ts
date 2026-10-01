// Stub for @tauri-apps/plugin-shell in demo mode
import { isValidHttpUrl } from '../../../src/utils/url'

export const open = async (url: string) => {
  if (!isValidHttpUrl(url)) throw new Error('Only HTTP and HTTPS links can be opened')
  window.open(url, '_blank', 'noopener,noreferrer')
}

export class Command {
  static create() {
    return new Command()
  }
  async spawn() {
    return { pid: 0 }
  }
  async execute() {
    return { code: 0, stdout: '', stderr: '' }
  }
}
