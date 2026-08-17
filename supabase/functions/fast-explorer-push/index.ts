import 'jsr:@supabase/functions-js/edge-runtime.d.ts'
import { createClient } from 'npm:@supabase/supabase-js@2'

type TransferRecord = {
  id: string
  receiver_device_id: string
  kind: 'file' | 'clipboard'
}

type WebhookPayload = {
  type: string
  table: string
  schema: string
  record: TransferRecord
}

type ServiceAccount = {
  project_id: string
  client_email: string
  private_key: string
}

const ACTION_OPEN_DEVICE_SYNC =
  'dev.oligami.fastexplorer.action.OPEN_DEVICE_SYNC'
const CHANNEL_ID = 'fast_explorer_sync'
const FCM_SCOPE = 'https://www.googleapis.com/auth/firebase.messaging'

Deno.serve(async (request) => {
  if (request.method !== 'POST') {
    return new Response('method not allowed', { status: 405 })
  }
  const expectedSecret = Deno.env.get('FASTEXPLORER_PUSH_WEBHOOK_SECRET') ?? ''
  if (!expectedSecret || request.headers.get('x-fast-explorer-webhook-secret') !== expectedSecret) {
    return new Response('unauthorized', { status: 401 })
  }

  const payload = await request.json() as WebhookPayload
  if (payload.type !== 'INSERT' || payload.table !== 'fast_explorer_transfers') {
    return Response.json({ skipped: true })
  }

  const supabaseUrl = mustEnv('SUPABASE_URL')
  const serviceRoleKey = mustEnv('SUPABASE_SERVICE_ROLE_KEY')
  const supabase = createClient(supabaseUrl, serviceRoleKey, {
    auth: { persistSession: false, autoRefreshToken: false },
  })
  const { data: device, error } = await supabase
    .from('fast_explorer_devices')
    .select('push_provider,push_token')
    .eq('id', payload.record.receiver_device_id)
    .maybeSingle()
  if (error) throw error
  if (!device?.push_token || device.push_provider !== 'fcm') {
    return new Response(null, { status: 204 })
  }

  const serviceAccount = JSON.parse(
    mustEnv('FIREBASE_SERVICE_ACCOUNT_JSON'),
  ) as ServiceAccount
  const accessToken = await googleAccessToken(serviceAccount)
  const title = 'FastExplorer device transfer'
  const body = payload.record.kind === 'file'
    ? 'A file is ready to receive'
    : 'A clipboard item is ready to receive'
  const response = await fetch(
    `https://fcm.googleapis.com/v1/projects/${serviceAccount.project_id}/messages:send`,
    {
      method: 'POST',
      headers: {
        authorization: `Bearer ${accessToken}`,
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        message: {
          token: device.push_token,
          notification: { title, body },
          data: {
            transfer_id: payload.record.id,
            kind: payload.record.kind,
          },
          android: {
            priority: 'high',
            collapse_key: 'fast_explorer_sync',
            notification: {
              channel_id: CHANNEL_ID,
              click_action: ACTION_OPEN_DEVICE_SYNC,
            },
          },
        },
      }),
    },
  )
  if (response.ok) {
    return Response.json({ sent: true })
  }

  const errorBody = await response.text()
  if (response.status === 404 || errorBody.includes('UNREGISTERED')) {
    await supabase.from('fast_explorer_devices')
      .update({ push_provider: null, push_token: null })
      .eq('id', payload.record.receiver_device_id)
  }
  return new Response(errorBody, { status: response.status })
})
function mustEnv(name: string): string {
  const value = Deno.env.get(name)?.trim()
  if (!value) throw new Error(`${name} is required`)
  return value
}

async function googleAccessToken(account: ServiceAccount): Promise<string> {
  const now = Math.floor(Date.now() / 1000)
  const header = base64UrlJson({ alg: 'RS256', typ: 'JWT' })
  const claims = base64UrlJson({
    iss: account.client_email,
    scope: FCM_SCOPE,
    aud: 'https://oauth2.googleapis.com/token',
    iat: now,
    exp: now + 3600,
  })
  const unsigned = `${header}.${claims}`
  const key = await importPrivateKey(account.private_key)
  const signature = await crypto.subtle.sign(
    'RSASSA-PKCS1-v1_5',
    key,
    new TextEncoder().encode(unsigned),
  )
  const assertion = `${unsigned}.${base64UrlBytes(new Uint8Array(signature))}`
  const form = new URLSearchParams({
    grant_type: 'urn:ietf:params:oauth:grant-type:jwt-bearer',
    assertion,
  })
  const response = await fetch('https://oauth2.googleapis.com/token', {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: form,
  })
  if (!response.ok) {
    throw new Error(`Google OAuth failed: ${response.status} ${await response.text()}`)
  }
  const body = await response.json() as { access_token?: string }
  if (!body.access_token) throw new Error('Google OAuth returned no access token')
  return body.access_token
}

async function importPrivateKey(pem: string): Promise<CryptoKey> {
  const base64 = pem
    .replace(/-----BEGIN PRIVATE KEY-----/g, '')
    .replace(/-----END PRIVATE KEY-----/g, '')
    .replace(/\s/g, '')
  const bytes = Uint8Array.from(atob(base64), (char) => char.charCodeAt(0))
  return crypto.subtle.importKey(
    'pkcs8',
    bytes,
    { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' },
    false,
    ['sign'],
  )
}

function base64UrlJson(value: unknown): string {
  return base64UrlBytes(new TextEncoder().encode(JSON.stringify(value)))
}
function base64UrlBytes(bytes: Uint8Array): string {
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary)
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/g, '')
}
