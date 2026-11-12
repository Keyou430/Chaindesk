import {
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  CircularProgress,
  FormControlLabel,
  Paper,
  Stack,
  TextField,
  Typography,
} from '@mui/material'
import { open } from '@tauri-apps/plugin-dialog'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BasePage } from '@/components/base'
import type { ChainPhase, RulePreset } from '@/services/bindings'
import {
  cancelChainConnection,
  connectChain,
  disconnectChain,
  excludeChainFront,
  forgetChainAuthentication,
  getChainState,
  importChainFile,
  importChainHttps,
  saveChainExit,
  updateChainSubscription,
} from '@/services/cmds'
import { mutate } from '@/services/mutate'
import { useQuery } from '@/services/query-client'

type ExitDraft = {
  host: string
  port: string
  requiresAuthentication: boolean
  username: string
  password: string
  remember: boolean
}
const queryKey = 'getChainState'

const ChainPage = () => {
  const { t } = useTranslation()
  const { data, error } = useQuery({
    queryKey,
    queryFn: getChainState,
    refetchInterval: 1000,
    retry: false,
  })
  const [draft, setDraft] = useState<ExitDraft | null>(null)
  const [url, setUrl] = useState('')
  const [proxyPort, setProxyPort] = useState('')
  const [frontId, setFrontId] = useState<string | null>(null)
  const [preset, setPreset] = useState<RulePreset | null>(null)
  const [consent, setConsent] = useState<boolean | null>(null)
  const [replaceAuthentication, setReplaceAuthentication] = useState(false)
  const [pending, setPending] = useState(false)
  const phase = data?.phase ?? 'unconfigured'
  const phaseLabels: Record<ChainPhase, string> = {
    unconfigured: t('home.chain.phases.unconfigured'),
    ready: t('home.chain.phases.ready'),
    probing: t('home.chain.phases.probing'),
    connecting: t('home.chain.phases.connecting'),
    connected: t('home.chain.phases.connected'),
    degraded: t('home.chain.phases.degraded'),
    disconnecting: t('home.chain.phases.disconnecting'),
    disconnected: t('home.chain.phases.disconnected'),
    recoveryRequired: t('home.chain.phases.recoveryRequired'),
  }
  const working = phase === 'connected' || phase === 'degraded'
  const testing =
    phase === 'connecting' || phase === 'probing' || phase === 'disconnecting'
  const busy = pending || testing
  const fields: ExitDraft = draft ?? {
    host: data?.host ?? '',
    port: String(data?.port ?? 1080),
    requiresAuthentication: data?.authenticationRequired ?? false,
    username: '',
    password: '',
    remember: data?.rememberAuthentication ?? false,
  }
  const selectedId = frontId ?? data?.selectedFrontId ?? ''
  const selected = data?.fronts.find((node) => node.id === selectedId)
  const reuseAuthentication =
    fields.requiresAuthentication &&
    !replaceAuthentication &&
    data?.hasSessionAuthentication === true &&
    fields.host === data.host &&
    Number(fields.port) === data.port
  const allowProbe = consent ?? data?.probeConsent ?? false
  const runBackground = false
  const choosePreset = preset ?? data?.preset ?? 'allProxy'
  const updateField = <K extends keyof ExitDraft>(
    key: K,
    value: ExitDraft[K],
  ) => setDraft({ ...fields, [key]: value })

  const perform = async (operation: () => Promise<unknown>) => {
    setPending(true)
    try {
      const result = await mutate(operation, {
        id: 'chain-session',
        revalidate: [queryKey],
      })
      return result.ok
    } catch {
      return false
    } finally {
      setPending(false)
    }
  }
  const importFile = () =>
    perform(async () => {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: 'Clash/Mihomo YAML', extensions: ['yaml', 'yml'] }],
      })
      return typeof path === 'string' ? importChainFile(path) : getChainState()
    })
  const connect = async () => {
    const succeeded = await perform(async () => {
      await saveChainExit({
        host: fields.host,
        port: Number(fields.port),
        username: reuseAuthentication
          ? null
          : fields.requiresAuthentication
            ? fields.username
            : '',
        password: reuseAuthentication
          ? null
          : fields.requiresAuthentication
            ? fields.password
            : '',
        remember: fields.remember,
      })
      return connectChain({
        frontId: selectedId,
        preset: choosePreset,
        probeConsent: allowProbe,
        backgroundChecks: runBackground,
      })
    })
    if (succeeded) {
      setDraft(null)
      setReplaceAuthentication(false)
    }
  }
  const cancel = async () => {
    try {
      await mutate(cancelChainConnection, {
        id: 'chain-cancel',
        revalidate: [queryKey],
      })
    } catch {
      /* The shared mutation boundary displays the error. */
    }
  }
  if (!data)
    return (
      <BasePage title={t('home.chain.title')}>
        <Alert severity={error ? 'error' : 'info'}>
          {error
            ? t('home.chain.phases.recoveryRequired')
            : t('home.chain.loading')}
        </Alert>
      </BasePage>
    )

  return (
    <BasePage
      title={t('home.chain.title')}
      header={
        <Chip
          data-testid="chain-phase"
          label={phaseLabels[phase]}
          color={
            phase === 'connected'
              ? 'success'
              : phase === 'degraded' || phase === 'recoveryRequired'
                ? 'warning'
                : 'default'
          }
        />
      }
    >
      <Stack spacing={2}>
        {data.error && <Alert severity="error">{data.error.message}</Alert>}
        <Paper variant="outlined" sx={{ p: 2 }}>
          <Stack spacing={1.5}>
            <Typography variant="h6">{t('home.chain.subscription')}</Typography>
            <Typography variant="body2">
              {data.subscriptionName ?? t('home.chain.empty')}
              {data.subscriptionUpdatedAt
                ? ` · ${new Date(data.subscriptionUpdatedAt).toLocaleString()}`
                : ''}
              {data.subscriptionHost ? ` · ${data.subscriptionHost}` : ''}
            </Typography>
            <Stack direction="row" spacing={1}>
              <Button
                variant="outlined"
                disabled={busy || working}
                onClick={() => {
                  void importFile()
                }}
              >
                {t('home.chain.importFile')}
              </Button>
              <Button
                disabled={busy || working || !data.canUpdateSubscription}
                onClick={() => {
                  void perform(() =>
                    updateChainSubscription(
                      proxyPort ? Number(proxyPort) : null,
                    ),
                  )
                }}
              >
                {t('home.chain.update')}
              </Button>
            </Stack>
            <TextField
              label={t('home.chain.url')}
              type="password"
              autoComplete="off"
              value={url}
              disabled={busy || working}
              onChange={(event) => setUrl(event.target.value)}
              size="small"
            />
            <Stack direction="row" spacing={1}>
              <TextField
                label={t('home.chain.localProxy')}
                type="number"
                value={proxyPort}
                disabled={busy || working}
                onChange={(event) => setProxyPort(event.target.value)}
                size="small"
                slotProps={{ htmlInput: { min: 1, max: 65535 } }}
              />
              <Button
                disabled={busy || working || !url.trim()}
                onClick={() => {
                  void perform(() =>
                    importChainHttps(url, proxyPort ? Number(proxyPort) : null),
                  ).then((ok) => {
                    if (ok) setUrl('')
                  })
                }}
              >
                {t('home.chain.importUrl')}
              </Button>
            </Stack>
          </Stack>
        </Paper>
        <Paper variant="outlined" sx={{ p: 2 }}>
          <Stack spacing={1.5}>
            <Typography variant="h6">{t('home.chain.exit')}</Typography>
            <Stack direction="row" spacing={1}>
              <TextField
                label={t('home.chain.host')}
                value={fields.host}
                disabled={busy || working}
                onChange={(event) => updateField('host', event.target.value)}
                size="small"
                fullWidth
              />
              <TextField
                label={t('home.chain.port')}
                type="number"
                value={fields.port}
                disabled={busy || working}
                onChange={(event) => updateField('port', event.target.value)}
                size="small"
                sx={{ width: 140 }}
                slotProps={{ htmlInput: { min: 1, max: 65535 } }}
              />
            </Stack>
            <FormControlLabel
              control={
                <Checkbox
                  checked={fields.requiresAuthentication}
                  disabled={busy || working}
                  onChange={(_, checked) =>
                    updateField('requiresAuthentication', checked)
                  }
                />
              }
              label={t('home.chain.authentication')}
            />
            {fields.requiresAuthentication && (
              <>
                {data.hasSessionAuthentication && (
                  <FormControlLabel
                    control={
                      <Checkbox
                        checked={reuseAuthentication}
                        disabled={busy || working}
                        onChange={(_, checked) =>
                          setReplaceAuthentication(!checked)
                        }
                      />
                    }
                    label={t('home.chain.reuse')}
                  />
                )}
                {!reuseAuthentication && (
                  <Stack direction="row" spacing={1}>
                    <TextField
                      label={t('home.chain.username')}
                      value={fields.username}
                      autoComplete="off"
                      disabled={busy || working}
                      onChange={(event) =>
                        updateField('username', event.target.value)
                      }
                      size="small"
                      fullWidth
                    />
                    <TextField
                      label={t('home.chain.password')}
                      type="password"
                      value={fields.password}
                      autoComplete="new-password"
                      disabled={busy || working}
                      onChange={(event) =>
                        updateField('password', event.target.value)
                      }
                      size="small"
                      fullWidth
                    />
                  </Stack>
                )}
                <FormControlLabel
                  control={
                    <Checkbox
                      checked={fields.remember}
                      disabled={busy || working}
                      onChange={(_, checked) =>
                        updateField('remember', checked)
                      }
                    />
                  }
                  label={t('home.chain.remember')}
                />
                <Button
                  disabled={
                    busy ||
                    working ||
                    (!data.hasSavedAuthentication &&
                      data.error?.code !== 'CREDENTIAL_CLEANUP_PENDING')
                  }
                  onClick={() => {
                    void perform(forgetChainAuthentication).then((ok) => {
                      if (ok) {
                        setDraft(null)
                        setReplaceAuthentication(false)
                      }
                    })
                  }}
                >
                  {t('home.chain.forget')}
                </Button>
              </>
            )}
            <Typography variant="caption" color="text.secondary">
              {t('home.chain.privacy')}
            </Typography>
          </Stack>
        </Paper>
        <Paper variant="outlined" sx={{ p: 2 }}>
          <Stack spacing={1.5}>
            <TextField
              id="chain-front"
              label={t('home.chain.front')}
              select
              value={selectedId}
              disabled={busy || working}
              onChange={(event) => setFrontId(event.target.value)}
              size="small"
              slotProps={{ select: { native: true } }}
            >
              <option value="">{t('home.chain.choose')}</option>
              {data.fronts.map((node) => (
                <option key={node.id} value={node.id}>
                  {node.name} · {node.protocol} · {node.endpoint} ·{' '}
                  {node.id.slice(0, 6)}
                </option>
              ))}
            </TextField>
            {selected?.rejection && (
              <Alert severity="warning">{selected.rejection}</Alert>
            )}
            <Button
              disabled={busy || working || !selected}
              onClick={() => {
                if (selected)
                  void perform(() =>
                    excludeChainFront(selected.id, !selected.excluded),
                  )
              }}
            >
              {selected?.excluded
                ? t('home.chain.include')
                : t('home.chain.exclude')}
            </Button>
            <TextField
              label={t('home.chain.rules')}
              select
              value={choosePreset}
              disabled={busy || working}
              onChange={(event) => setPreset(event.target.value as RulePreset)}
              size="small"
              slotProps={{ select: { native: true } }}
            >
              <option value="allProxy">{t('home.chain.allProxy')}</option>
              <option value="chinaDirect">{t('home.chain.chinaDirect')}</option>
            </TextField>
            <FormControlLabel
              control={
                <Checkbox
                  id="chain-probe-consent"
                  checked={allowProbe}
                  disabled={busy || working}
                  onChange={(_, checked) => setConsent(checked)}
                />
              }
              label={t('home.chain.consent')}
            />

            <Stack direction="row" spacing={1}>
              <Button
                data-testid="chain-connect"
                variant="contained"
                disabled={
                  busy ||
                  working ||
                  !selected ||
                  !!selected.rejection ||
                  !allowProbe ||
                  !fields.host ||
                  !fields.port ||
                  (fields.requiresAuthentication &&
                    !reuseAuthentication &&
                    (!fields.username || !fields.password))
                }
                onClick={() => {
                  void connect()
                }}
              >
                {busy && <CircularProgress size={16} sx={{ mr: 1 }} />}
                {t('home.chain.connect')}
              </Button>
              <Button
                data-testid="chain-disconnect"
                disabled={busy || !working}
                onClick={() => {
                  void perform(disconnectChain)
                }}
              >
                {t('home.chain.disconnect')}
              </Button>
              {testing && (
                <Button
                  onClick={() => {
                    void cancel()
                  }}
                >
                  {t('home.chain.cancel')}
                </Button>
              )}
            </Stack>
            <Box>
              <Typography variant="body2">
                {t('home.chain.observed')}:{' '}
                {data.observation?.ipv4 ?? t('home.chain.notObserved')}
              </Typography>
              {data.observation && (
                <Typography variant="caption">
                  {new Date(data.observation.checkedAt).toLocaleString()} ·{' '}
                  {data.observation.service}
                </Typography>
              )}
            </Box>
            <Typography variant="caption" color="text.secondary">
              {t('home.chain.coverage')}
            </Typography>
          </Stack>
        </Paper>
      </Stack>
    </BasePage>
  )
}

export default ChainPage
