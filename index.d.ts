export type Format =
  | 'docx'
  | 'docm'
  | 'dotx'
  | 'dotm'
  | 'doc'
  | 'dot'
  | 'odt'
  | 'ott'
  | 'fodt'
  | 'ods'
  | 'ots'
  | 'fods'
  | 'odp'
  | 'otp'
  | 'fodp'
  | 'pptx'
  | 'xlsx'
  | 'xlsm'
  | 'xltx'
  | 'xltm'
  | 'xls'
  | 'xlt'
  | 'md'
  | 'markdown'

export interface ConvertOptions {
  to: Format | string
  from?: Format | string
  fontsDir?: string
}

export function convert(input: Uint8Array | ArrayBuffer, opts: ConvertOptions): Buffer

export function detect(input: Uint8Array | ArrayBuffer): Format | null

export function supports(from: string, to: string): boolean

export function conversions(): Array<{ from: string; to: string }>

export const FONTS_DIR: string
