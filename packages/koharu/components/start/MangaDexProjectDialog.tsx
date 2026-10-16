'use client'

import { Channel } from '@tauri-apps/api/core'
import { CircleStop, Download, FolderOpen, Search } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { commands } from '@koharu/bridge/protocol'
import { Alert, AlertDescription, AlertTitle } from '@koharu/ui/components/alert'
import { Button } from '@koharu/ui/components/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@koharu/ui/components/dialog'
import { Input } from '@koharu/ui/components/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'

type MangaDexLanguage = { code: string; name: string }

type MangaDexImportEvent = {
  job_id: string
  state: 'runtime_setup' | 'downloading' | 'importing' | 'finished' | 'failed' | 'cancelled'
  error: string | null
}

type MangaDexCommands = {
  resolveMangadex: (url: string) => Promise<{
    manga_id: string
    title: string
    languages: MangaDexLanguage[]
  }>
  pickMangadexOutputFolder: () => Promise<string | null>
  startMangadexImport: (
    name: string,
    url: string,
    language: string,
    output: string,
    onEvent: Channel<MangaDexImportEvent>,
  ) => Promise<string>
  cancelMangadexImport: (jobId: string) => Promise<null>
}

const mangadex = commands as typeof commands & MangaDexCommands

const activeStates = new Set(['runtime_setup', 'downloading', 'importing', 'cancelling'])

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export function MangaDexProjectDialog({
  open,
  onOpenChange,
  onSuccess,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSuccess: () => Promise<void>
}) {
  const { t } = useTranslation()
  const [url, setUrl] = useState('')
  const [name, setName] = useState('')
  const [languages, setLanguages] = useState<MangaDexLanguage[]>([])
  const [language, setLanguage] = useState<string | null>(null)
  const [output, setOutput] = useState<string | null>(null)
  const [status, setStatus] = useState<string | null>(null)
  const [jobId, setJobId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [destinationCancelled, setDestinationCancelled] = useState(false)

  const active = status !== null && activeStates.has(status)
  const canStart = Boolean(
    name.trim() && url.trim() && language && output && !active && status !== 'resolving',
  )

  async function resolve() {
    const mangaUrl = url.trim()
    if (!mangaUrl || active) return
    setError(null)
    setStatus('resolving')
    setLanguages([])
    setLanguage(null)
    try {
      const metadata = await call(mangadex.resolveMangadex, mangaUrl)
      setName(metadata.title)
      setLanguages(metadata.languages)
      setLanguage(metadata.languages[0]?.code ?? null)
      if (metadata.languages.length === 0) setError(t('mangadex.noLanguages'))
    } catch (error) {
      setError(errorMessage(error))
    } finally {
      setStatus(null)
    }
  }

  async function chooseDestination() {
    if (active) return
    setError(null)
    setDestinationCancelled(false)
    try {
      const selected = await call(mangadex.pickMangadexOutputFolder)
      if (selected) setOutput(selected)
      else setDestinationCancelled(true)
    } catch (error) {
      setError(errorMessage(error))
    }
  }

  async function start() {
    if (!canStart || !language || !output) return
    setError(null)
    setDestinationCancelled(false)
    setStatus('runtime_setup')
    const channel = new Channel<MangaDexImportEvent>()
    channel.onmessage = (event) => {
      setJobId(event.job_id)
      setStatus(event.state)
      if (event.state === 'failed') setError(event.error ?? t('mangadex.importFailed'))
      if (event.state === 'finished') void onSuccess()
    }
    try {
      const started = await call(
        mangadex.startMangadexImport,
        name.trim(),
        url.trim(),
        language,
        output,
        channel,
      )
      setJobId((current) => current ?? started)
    } catch (error) {
      setStatus('failed')
      setError(errorMessage(error))
    }
  }

  async function cancel() {
    if (!jobId || !active) return
    setError(null)
    setStatus('cancelling')
    try {
      await call(mangadex.cancelMangadexImport, jobId)
    } catch (error) {
      setStatus('failed')
      setError(errorMessage(error))
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (nextOpen || !active) onOpenChange(nextOpen)
      }}
    >
      <DialogContent className='max-w-md gap-4 p-5'>
        <DialogHeader>
          <DialogTitle>{t('mangadex.title')}</DialogTitle>
          <DialogDescription>{t('mangadex.description')}</DialogDescription>
        </DialogHeader>

        <div className='grid gap-3'>
          <label className='grid gap-1.5 text-[11px] font-medium' htmlFor='mangadex-url'>
            {t('mangadex.url')}
            <div className='flex gap-2'>
              <Input
                id='mangadex-url'
                value={url}
                onChange={(event) => setUrl(event.target.value)}
                placeholder={t('mangadex.urlPlaceholder')}
                disabled={active}
                autoComplete='off'
                className='h-9 text-[11px]'
              />
              <Button
                type='button'
                size='sm'
                variant='outline'
                disabled={!url.trim() || active || status === 'resolving'}
                onClick={() => void resolve()}
              >
                <Search className='size-3.5' />
                {t('mangadex.resolve')}
              </Button>
            </div>
          </label>

          <label className='grid gap-1.5 text-[11px] font-medium' htmlFor='mangadex-project-name'>
            {t('start.projectName')}
            <Input
              id='mangadex-project-name'
              value={name}
              onChange={(event) => setName(event.target.value)}
              disabled={active}
              autoComplete='off'
              className='h-9 text-[11px]'
            />
          </label>

          <label className='grid gap-1.5 text-[11px] font-medium' htmlFor='mangadex-language'>
            {t('mangadex.language')}
            <Select
              value={language ?? undefined}
              onValueChange={setLanguage}
              disabled={active || !languages.length}
            >
              <SelectTrigger
                id='mangadex-language'
                className='h-9 w-full text-[11px]'
                aria-label={t('mangadex.language')}
              >
                <SelectValue placeholder={t('mangadex.languagePlaceholder')} />
              </SelectTrigger>
              <SelectContent>
                {languages.map((item) => (
                  <SelectItem key={item.code} value={item.code}>
                    {item.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>

          <div className='grid gap-1.5 text-[11px] font-medium'>
            <span>{t('mangadex.destination')}</span>
            <div className='flex gap-2'>
              <Input
                value={output ?? ''}
                readOnly
                aria-label={t('mangadex.destination')}
                placeholder={t('mangadex.destinationPlaceholder')}
                className='h-9 text-[11px]'
              />
              <Button
                type='button'
                size='sm'
                variant='outline'
                disabled={active}
                onClick={() => void chooseDestination()}
              >
                <FolderOpen className='size-3.5' />
                {t('mangadex.chooseDestination')}
              </Button>
            </div>
            {destinationCancelled ? (
              <p className='text-[10px] font-normal text-muted-foreground' role='status'>
                {t('mangadex.destinationCancelled')}
              </p>
            ) : null}
          </div>
        </div>

        {status && status !== 'resolving' ? (
          <p className='text-[11px] text-muted-foreground' role='status'>
            {t(`mangadex.status.${status}`)}
          </p>
        ) : null}
        {error ? (
          <Alert variant='destructive'>
            <AlertTitle>{t('mangadex.errorTitle')}</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        ) : null}

        <DialogFooter>
          {active ? (
            <Button
              type='button'
              variant='outline'
              onClick={() => void cancel()}
              disabled={!jobId || status === 'cancelling'}
            >
              <CircleStop className='size-3.5' />
              {t('mangadex.cancelImport')}
            </Button>
          ) : (
            <Button type='button' variant='outline' onClick={() => onOpenChange(false)}>
              {t('common.cancel')}
            </Button>
          )}
          <Button type='button' disabled={!canStart} onClick={() => void start()}>
            <Download className='size-3.5' />
            {t('mangadex.startImport')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
