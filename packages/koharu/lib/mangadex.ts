import { invoke, type Channel } from '@tauri-apps/api/core'

export type MangaDexLanguage = { code: string; name: string }

export type MangaDexMetadata = {
  manga_id: string
  title: string
  languages: MangaDexLanguage[]
}

export type MangaDexImportEvent = {
  job_id: string
  state: 'runtime_setup' | 'downloading' | 'importing' | 'finished' | 'failed' | 'cancelled'
  error: string | null
}

export const mangaDexCommands = {
  resolve: (url: string) => invoke<MangaDexMetadata>('resolve_mangadex', { url }),
  pickOutputFolder: () => invoke<string | null>('pick_mangadex_output_folder'),
  startImport: (
    name: string,
    url: string,
    language: string,
    output: string,
    onEvent: Channel<MangaDexImportEvent>,
  ) => invoke<string>('start_mangadex_import', { name, url, language, output, onEvent }),
  cancelImport: (jobId: string) => invoke<null>('cancel_mangadex_import', { jobId }),
}
