// Stub for @tauri-apps/plugin-shell in demo mode
export const open = async (url: string) => {
  window.open(url, '_blank')
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
