import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, it, vi } from 'vitest'

import { AgentPanel } from '@/components/editor/AgentPanel'
import { useKoharuStore } from '@/lib/store'
import { commands, type AgentStatus, type Model } from '@koharu/bridge/protocol'

vi.mock('@tauri-apps/api/core', () => ({
  Channel: class {
    onmessage = () => undefined
  },
  invoke: vi.fn(),
}))

const signedOut: AgentStatus = {
  account: null,
  models: [],
  config: { model: null, reasoning: 'medium' },
  running: null,
}

const signedIn: AgentStatus = {
  ...signedOut,
  account: { id: 'account', email: 'reader@example.com', plan: 'plus' },
  models: [{ id: 'codex', name: 'Codex', reasoning: ['low', 'medium'], vision: true }],
}

const translationModel: Model = {
  provider: 'codex',
  model: 'codex',
  name: 'Codex',
  quantizations: [],
  vision: true,
  reasoning: true,
}

beforeEach(() => {
  vi.spyOn(commands, 'getAgentStatus').mockResolvedValue(signedOut)
  vi.spyOn(commands, 'loginAgent').mockResolvedValue(signedIn)
  vi.spyOn(commands, 'logoutAgent').mockResolvedValue(signedOut)
  vi.spyOn(commands, 'getTranslationModels').mockResolvedValue([translationModel])
})

it('makes Codex translation models available after signing in', async () => {
  render(<AgentPanel />)
  const user = userEvent.setup()
  await user.click(await screen.findByRole('button', { name: 'Sign in with Codex' }))

  await waitFor(() =>
    expect(useKoharuStore.getState().translationModels).toEqual([translationModel]),
  )
})

it('removes Codex translation models after signing out', async () => {
  vi.mocked(commands.getAgentStatus).mockResolvedValue(signedIn)
  vi.mocked(commands.getTranslationModels).mockResolvedValue([])
  useKoharuStore.setState({ translationModels: [translationModel] })
  render(<AgentPanel />)
  const user = userEvent.setup()
  await user.click(await screen.findByRole('button', { name: 'Sign out' }))

  await waitFor(() => expect(useKoharuStore.getState().translationModels).toEqual([]))
})
