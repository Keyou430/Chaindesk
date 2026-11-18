import {
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  Paper,
  Stack,
  TextField,
  Typography,
} from '@mui/material'
import { open, save } from '@tauri-apps/plugin-dialog'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BasePage } from '@/components/base'
import type {
  ChainPhase,
  DiagnosticPreview,
  RulePreset,
} from '@/services/bindings'
import {
  cancelChainConnection,
  connectChain,
  disconnectChain,
  excludeChainFront,
  forgetChainAuthentication,
  getChainState,
  importChainFile,
  importChainHttps,
  restoreChainLastGood,
  confirmChainIpv4,
  previewChainDiagnostics,
  exportChainDiagnostics,
  keepChainCurrentProxy,
  setChainBackgroundChecks,
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
  const [automatic, setAutomatic] = useState<boolean | null>(null)
  const [closeExisting, setCloseExisting] = useState(false)
  const [background, setBackground] = useState<boolean | null>(null)
  const [expected, setExpected] = useState<string | null>(null)
  const [diagnostic, setDiagnostic] = useState<DiagnosticPreview | null>(null)
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
  const automaticFront = automatic ?? data?.automaticFront ?? false
  const selected = data?.fronts.find((node) => node.id === selectedId)
  const verifiedFront = data?.fronts.find(
    (node) => node.id === data.selectedFrontId,
  )
  const reuseAuthentication =
    fields.requiresAuthentication &&
    !replaceAuthentication &&
    data?.hasSessionAuthentication === true &&
    fields.host === data.host &&
    Number(fields.port) === data.port
  const allowProbe = consent ?? data?.probeConsent ?? false
  const runBackground =
    (working
      ? data?.backgroundChecks
      : (background ?? data?.backgroundChecks)) ?? true
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
      return connectChain({
        frontId: selectedId,
        preset: choosePreset,
        probeConsent: allowProbe,
        backgroundChecks: runBackground,
        automaticFront,
        closeExistingConnections: closeExisting,
        expectedIpv4: expected,
        exit: {
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
        },
      })
    })
    if (succeeded) {
      setFrontId(null)
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
        {data.error && (
          <Alert
            severity={
              ['EXIT_IP_INCONSISTENT', 'EXIT_IP_UNAVAILABLE'].includes(
                data.error.code,
              ) && phase === 'connected'
                ? 'warning'
                : 'error'
            }
          >
            {data.error.message}
          </Alert>
        )}
        {data.candidateObservation && (
          <Alert severity="warning">
            {t('home.chain.candidateObservation')}:{' '}
            {data.candidateObservation.sources
              .map(
                (source) =>
                  `${source.ipv4} · ${new Date(source.checkedAt).toLocaleString()} · ${source.service}`,
              )
              .join(' / ')}
          </Alert>
        )}
        {data.error?.code === 'RESTORE_CONFLICT' && (
          <Button
            disabled={busy}
            onClick={() => {
              void perform(keepChainCurrentProxy)
            }}
          >
            {t('home.chain.keepCurrent')}
          </Button>
        )}
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
                disabled={busy}
                onChange={(event) => updateField('host', event.target.value)}
                size="small"
                fullWidth
              />
              <TextField
                label={t('home.chain.port')}
                type="number"
                value={fields.port}
                disabled={busy}
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
                  disabled={busy}
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
                        disabled={busy}
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
                      disabled={busy}
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
                      disabled={busy}
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
                      disabled={busy}
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
              disabled={busy}
              onChange={(event) => setFrontId(event.target.value)}
              size="small"
              helperText={
                working && frontId && frontId !== data.selectedFrontId
                  ? t('home.chain.pendingFront')
                  : undefined
              }
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
            {working && verifiedFront && (
              <Typography
                variant="body2"
                data-testid="chain-active-front"
                data-front-id={verifiedFront.id}
              >
                {t('home.chain.currentFront')}: {verifiedFront.name} ·{' '}
                {verifiedFront.protocol} · {verifiedFront.endpoint} ·{' '}
                {verifiedFront.id.slice(0, 6)}
              </Typography>
            )}
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
              disabled={busy}
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

            <FormControlLabel
              control={
                <Checkbox
                  checked={automaticFront}
                  disabled={busy}
                  onChange={(_, value) => setAutomatic(value)}
                />
              }
              label={t('home.chain.automatic')}
            />
            <TextField
              label={t('home.chain.expected')}
              value={expected ?? data.expectedIpv4 ?? ''}
              disabled={busy}
              onChange={(event) => setExpected(event.target.value)}
              size="small"
            />
            <FormControlLabel
              control={
                <Checkbox
                  checked={runBackground}
                  disabled={busy || !allowProbe}
                  onChange={(_, value) => {
                    if (working) {
                      void perform(() => setChainBackgroundChecks(value))
                    } else {
                      setBackground(value)
                    }
                  }}
                />
              }
              label={t('home.chain.background')}
            />
            <Typography variant="caption" color="text.secondary">
              {t('home.chain.backgroundDetail')}
            </Typography>
            <Stack direction="row" spacing={1}>
              <Button
                data-testid="chain-connect"
                variant="contained"
                disabled={
                  busy ||
                  (!automaticFront && (!selected || !!selected.rejection)) ||
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
                {working ? t('home.chain.switch') : t('home.chain.connect')}
              </Button>
              <Button
                data-testid="chain-disconnect"
                disabled={busy || (!working && phase !== 'recoveryRequired')}
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
              {data.healthCheckedAt && (
                <Typography variant="caption" sx={{ display: 'block' }}>
                  {t('home.chain.lastHealthCheck')}:{' '}
                  {new Date(data.healthCheckedAt).toLocaleString()}
                </Typography>
              )}
              <Typography variant="body2">
                {t('home.chain.observed')}:{' '}
                {data.observation?.ipv4 ?? t('home.chain.notObserved')}
              </Typography>
              {data.observation?.sources.map((source) => (
                <Typography
                  variant="caption"
                  sx={{ display: 'block' }}
                  key={source.service}
                >
                  {source.ipv4} · {new Date(source.checkedAt).toLocaleString()}{' '}
                  · {source.service}
                </Typography>
              ))}
              <Button
                disabled={
                  busy ||
                  !data.observation?.consistent ||
                  data.observation.sources.length !== 2
                }
                onClick={() => {
                  if (data.observation) {
                    const ipv4 = data.observation.ipv4
                    void perform(() => confirmChainIpv4(ipv4)).then((ok) => {
                      if (ok) setExpected(null)
                    })
                  }
                }}
              >
                {t('home.chain.baseline')}
              </Button>
              <Button
                disabled={busy || working || !data.canRestoreLastGood}
                onClick={() => {
                  void perform(restoreChainLastGood).then((ok) => {
                    if (ok) {
                      setDraft(null)
                      setFrontId(null)
                      setPreset(null)
                      setExpected(null)
                      setConsent(null)
                      setAutomatic(null)
                      setBackground(null)
                      setReplaceAuthentication(false)
                    }
                  })
                }}
              >
                {t('home.chain.restore')}
              </Button>
              <Button
                disabled={busy}
                onClick={() => {
                  void perform(async () => {
                    setDiagnostic(await previewChainDiagnostics())
                  })
                }}
              >
                {t('home.chain.diagnostics')}
              </Button>
            </Box>
            <Typography variant="caption" color="text.secondary">
              {t('home.chain.switchNotice')}
            </Typography>
            <FormControlLabel
              control={
                <Checkbox
                  checked={closeExisting}
                  disabled={busy || !working}
                  onChange={(_, value) => setCloseExisting(value)}
                />
              }
              label={t('home.chain.closeExisting')}
            />
            <Typography variant="caption" color="text.secondary">
              {t('home.chain.coverage')}
            </Typography>
          </Stack>
        </Paper>
      </Stack>
      <Dialog
        open={diagnostic !== null}
        onClose={() => setDiagnostic(null)}
        maxWidth="md"
        fullWidth
      >
        <DialogTitle>{t('home.chain.diagnostics')}</DialogTitle>
        <DialogContent>
          <Typography variant="body2">
            {t('home.chain.diagnosticPrivacy')}
          </Typography>
          <Box component="pre" sx={{ overflow: 'auto', fontSize: 12 }}>
            {diagnostic?.json}
          </Box>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setDiagnostic(null)}>
            {t('home.chain.close')}
          </Button>
          <Button
            disabled={pending || !diagnostic}
            onClick={() => {
              void perform(async () => {
                const path = await save({
                  defaultPath: `ChainDesk-diagnostics-${Date.now()}.json`,
                  filters: [{ name: 'JSON', extensions: ['json'] }],
                })
                if (path && diagnostic)
                  await exportChainDiagnostics(path, diagnostic.sha256)
              })
            }}
          >
            {t('home.chain.exportDiagnostics')}
          </Button>
        </DialogActions>
      </Dialog>
    </BasePage>
  )
}

export default ChainPage
