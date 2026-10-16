import { QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { StartView } from '@/components/start/StartView'
import { queryClient, useProject } from '@/lib/queries'
import { commands, type MangaDexImportEvent } from '@koharu/bridge/protocol'

type MangaDexEvent = MangaDexImportEvent

function ProjectFlow() {
  const project = useProject().data
  return project ? <p>Opened {project.name}</p> : <StartView />
}

function renderProjectFlow() {
  return render(
    <QueryClientProvider client={queryClient}>
      <ProjectFlow />
    </QueryClientProvider>,
  )
}

describe('StartView', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  function setupMangaDex() {
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      configurable: true,
      value: { transformCallback: vi.fn(() => 'mangadex-channel') },
    })
    vi.spyOn(commands, 'listProjects').mockResolvedValue([])
    const resolveMangadex = vi.spyOn(commands, 'resolveMangadex')
    const pickMangadexOutputFolder = vi.spyOn(commands, 'pickMangadexOutputFolder')
    const startMangadexImport = vi.spyOn(commands, 'startMangadexImport')
    const cancelMangadexImport = vi.spyOn(commands, 'cancelMangadexImport')
    return { resolveMangadex, pickMangadexOutputFolder, startMangadexImport, cancelMangadexImport }
  }

  async function openMangaDexDialog() {
    const open = screen.getByRole('button', { name: 'Import from MangaDex' })
    await waitFor(() => expect(open).toBeEnabled())
    fireEvent.click(open)
    return screen.findByRole('dialog', { name: 'Import from MangaDex' })
  }

  async function resolveManga(title = 'Blue Period') {
    const url = 'https://mangadex.org/title/12345678-1234-1234-1234-123456789abc'
    const mocks = setupMangaDex()
    mocks.resolveMangadex.mockResolvedValue({
      manga_id: '12345678-1234-1234-1234-123456789abc',
      title,
      languages: [
        { code: 'en', name: 'English' },
        { code: 'ja', name: 'Japanese' },
      ],
    })
    render(<StartView />)
    const dialog = await openMangaDexDialog()
    fireEvent.change(screen.getByRole('textbox', { name: 'MangaDex URL' }), {
      target: { value: url },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Find manga' }))
    await waitFor(() => expect(mocks.resolveMangadex).toHaveBeenCalledWith(url))
    await screen.findByDisplayValue(title)
    return { dialog, mocks, url }
  }

  it('creates a managed project by name', async () => {
    let opened = false
    vi.spyOn(commands, 'getProject').mockImplementation(async () =>
      opened
        ? {
            name: 'Volume 1',
            revision: 0,
            active_page: null,
            can_undo: false,
            can_redo: false,
          }
        : null,
    )
    vi.spyOn(commands, 'listProjects').mockResolvedValue([])
    const create = vi.spyOn(commands, 'createProject').mockImplementation(async () => {
      opened = true
      return null
    })
    renderProjectFlow()
    expect(await screen.findByRole('heading', { name: 'Projects' })).toBeInTheDocument()
    fireEvent.change(screen.getByRole('textbox', { name: 'Project name' }), {
      target: { value: 'Volume 1' },
    })
    const createButton = screen.getByRole('button', { name: 'Create' })
    await waitFor(() => expect(createButton).toBeEnabled())
    fireEvent.click(createButton)
    await waitFor(() => expect(create).toHaveBeenCalledWith('Volume 1'))
    expect(await screen.findByText('Opened Volume 1')).toBeInTheDocument()
  })

  it('opens a managed project without reloading the application', async () => {
    let opened = false
    vi.spyOn(commands, 'getProject').mockImplementation(async () =>
      opened
        ? {
            name: 'Blue Archive',
            revision: 1,
            active_page: null,
            can_undo: false,
            can_redo: false,
          }
        : null,
    )
    vi.spyOn(commands, 'listProjects').mockResolvedValue([{ name: 'Blue Archive' }])
    const open = vi.spyOn(commands, 'openProject').mockImplementation(async () => {
      opened = true
      return null
    })
    renderProjectFlow()

    const project = (await screen.findByText('Blue Archive')).closest('button')
    if (!project) throw new Error('project row is not interactive')
    await waitFor(() => expect(project).toBeEnabled())
    fireEvent.click(project)

    await waitFor(() => expect(open).toHaveBeenCalledWith('Blue Archive'))
    expect(await screen.findByText('Opened Blue Archive')).toBeInTheDocument()
  })

  it('confirms before deleting a managed project', async () => {
    vi.spyOn(commands, 'listProjects')
      .mockResolvedValueOnce([{ name: 'Blue Archive' }])
      .mockResolvedValueOnce([])
    const remove = vi.spyOn(commands, 'deleteProject').mockResolvedValue(null)
    render(<StartView />)

    const deleteButton = await screen.findByRole('button', { name: 'Delete Blue Archive' })
    await waitFor(() => expect(deleteButton).toBeEnabled())
    fireEvent.click(deleteButton)

    expect(screen.getByRole('alertdialog')).toHaveTextContent(
      'This permanently deletes “Blue Archive” and all of its pages.',
    )
    expect(remove).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole('button', { name: 'Delete project' }))
    await waitFor(() => expect(remove).toHaveBeenCalledWith('Blue Archive'))
  })

  it('prefills the MangaDex title while keeping the project name editable', async () => {
    const { dialog } = await resolveManga()
    const projectName = within(dialog).getByRole('textbox', { name: 'Project name' })

    expect(projectName).toHaveValue('Blue Period')
    fireEvent.change(projectName, { target: { value: 'Blue Period volume 1' } })
    expect(projectName).toHaveValue('Blue Period volume 1')
  })

  it('lets the user choose an available chapter language', async () => {
    const { dialog } = await resolveManga()
    const user = userEvent.setup()

    await user.click(within(dialog).getByRole('combobox', { name: 'Chapter language' }))
    await user.click(await screen.findByRole('option', { name: 'Japanese' }))
    expect(within(dialog).getByRole('combobox', { name: 'Chapter language' })).toHaveTextContent(
      'Japanese',
    )
  })

  it('requires a destination before the MangaDex import can start', async () => {
    const { dialog, mocks } = await resolveManga()
    const start = within(dialog).getByRole('button', { name: 'Import chapters' })

    expect(start).toBeDisabled()
    mocks.pickMangadexOutputFolder.mockResolvedValue('/Users/example/Downloads')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Choose destination' }))
    await waitFor(() => expect(start).toBeEnabled())
  })

  it('requires resolving the current URL before starting an import', async () => {
    const { dialog, mocks } = await resolveManga()
    mocks.pickMangadexOutputFolder.mockResolvedValue('/Users/example/Downloads')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Choose destination' }))
    const start = within(dialog).getByRole('button', { name: 'Import chapters' })
    await waitFor(() => expect(start).toBeEnabled())

    fireEvent.change(within(dialog).getByRole('textbox', { name: 'MangaDex URL' }), {
      target: { value: 'https://mangadex.org/title/87654321-1234-1234-1234-123456789abc' },
    })

    expect(start).toBeDisabled()
    fireEvent.click(start)
    expect(mocks.startMangadexImport).not.toHaveBeenCalled()
  })

  it('does not start stale metadata while resolving a replacement MangaDex URL', async () => {
    const { dialog, mocks } = await resolveManga()
    mocks.pickMangadexOutputFolder.mockResolvedValue('/Users/example/Downloads')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Choose destination' }))
    const start = within(dialog).getByRole('button', { name: 'Import chapters' })
    await waitFor(() => expect(start).toBeEnabled())

    let resolveReplacement:
      | ((value: Awaited<ReturnType<typeof commands.resolveMangadex>>) => void)
      | null = null
    mocks.resolveMangadex.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveReplacement = resolve
        }),
    )
    fireEvent.change(within(dialog).getByRole('textbox', { name: 'MangaDex URL' }), {
      target: { value: 'https://mangadex.org/title/87654321-1234-1234-1234-123456789abc' },
    })
    fireEvent.click(within(dialog).getByRole('button', { name: 'Find manga' }))

    expect(start).toBeDisabled()
    fireEvent.click(start)
    expect(mocks.startMangadexImport).not.toHaveBeenCalled()

    resolveReplacement?.({
      manga_id: '87654321-1234-1234-1234-123456789abc',
      title: 'New title',
      languages: [],
    })
    await screen.findByDisplayValue('New title')
  })

  it('shows native import progress and lets the user cancel an active import', async () => {
    const { dialog, mocks, url } = await resolveManga()
    mocks.pickMangadexOutputFolder.mockResolvedValue('/Users/example/Downloads')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Choose destination' }))
    await screen.findByDisplayValue('/Users/example/Downloads')
    let emit: ((event: MangaDexEvent) => void) | null = null
    mocks.startMangadexImport.mockImplementation(
      async (_name, _url, _language, _output, channel) => {
        emit = channel.onmessage
        return 'import-1'
      },
    )
    mocks.cancelMangadexImport.mockResolvedValue(null)

    fireEvent.click(within(dialog).getByRole('button', { name: 'Import chapters' }))
    await waitFor(() =>
      expect(mocks.startMangadexImport).toHaveBeenCalledWith(
        'Blue Period',
        url,
        'en',
        '/Users/example/Downloads',
        expect.anything(),
      ),
    )
    emit?.({ job_id: 'import-1', state: 'downloading', error: null })
    expect(await within(dialog).findByText('Downloading chapters…')).toBeInTheDocument()

    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel import' }))
    await waitFor(() => expect(mocks.cancelMangadexImport).toHaveBeenCalledWith('import-1'))
    emit?.({ job_id: 'import-1', state: 'cancelled', error: null })
    expect(await within(dialog).findByText('Import cancelled.')).toBeInTheDocument()
  })

  it('shows actionable MangaDex errors', async () => {
    const mocks = setupMangaDex()
    mocks.resolveMangadex.mockRejectedValue(new Error('MangaDex URL must use HTTPS'))
    render(<StartView />)
    await openMangaDexDialog()
    fireEvent.change(screen.getByRole('textbox', { name: 'MangaDex URL' }), {
      target: { value: 'http://mangadex.org/title/example' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Find manga' }))

    expect(await screen.findByRole('alert')).toHaveTextContent('MangaDex URL must use HTTPS')
  })
})
