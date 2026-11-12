import { Alert, Stack } from '@mui/material'
import { useTranslation } from 'react-i18next'

import { BasePage } from '@/components/base'
import SettingClash from '@/components/setting/setting-clash'
import SettingVergeBasic from '@/components/setting/setting-verge-basic'
import { showNotice } from '@/services/notice-service'

const PortableSettingsPage = () => {
  const { t } = useTranslation()
  return (
    <BasePage title={t('home.chain.settings')}>
      <Stack spacing={2}>
        <SettingVergeBasic
          compact
          onError={(error) => showNotice.error(error)}
        />
        <SettingClash onError={(error) => showNotice.error(error)} />
        <Alert severity="info">{t('home.chain.privacy')}</Alert>
      </Stack>
    </BasePage>
  )
}

export default PortableSettingsPage
