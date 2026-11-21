package dev.chaindesk.android.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

private val daylight = lightColorScheme(
    primary = Color(0xFF006D67),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFDCEEE9),
    onPrimaryContainer = Color(0xFF123F3D),
    secondary = Color(0xFF4F626A),
    background = Color(0xFFF7F8F5),
    onBackground = Color(0xFF172C37),
    surface = Color(0xFFF7F8F5),
    onSurface = Color(0xFF172C37),
    surfaceVariant = Color(0xFFEBEFEB),
    onSurfaceVariant = Color(0xFF53636A),
    outline = Color(0xFF788883),
    outlineVariant = Color(0xFFD6DEDA),
    error = Color(0xFF9D352F),
    errorContainer = Color(0xFFF8E7E3),
    onErrorContainer = Color(0xFF702C27),
)

private val night = darkColorScheme(
    primary = Color(0xFF86D0C3),
    onPrimary = Color(0xFF003733),
    primaryContainer = Color(0xFF183F3D),
    onPrimaryContainer = Color(0xFFBDECE1),
    secondary = Color(0xFFB0C6CC),
    background = Color(0xFF111D24),
    onBackground = Color(0xFFDFE8E8),
    surface = Color(0xFF111D24),
    onSurface = Color(0xFFDFE8E8),
    surfaceVariant = Color(0xFF213239),
    onSurfaceVariant = Color(0xFFB2C2C3),
    outline = Color(0xFF7F9492),
    outlineVariant = Color(0xFF34484A),
    error = Color(0xFFFFB4A9),
    errorContainer = Color(0xFF512D2A),
    onErrorContainer = Color(0xFFFFDAD3),
)

private val chainTypography = Typography(
    headlineLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.SemiBold, fontSize = 30.sp, lineHeight = 40.sp),
    headlineSmall = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.SemiBold, fontSize = 24.sp, lineHeight = 32.sp),
    titleLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.SemiBold, fontSize = 20.sp, lineHeight = 28.sp),
    titleMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.Medium, fontSize = 16.sp, lineHeight = 24.sp),
    bodyLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 16.sp, lineHeight = 25.sp),
    bodyMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 14.sp, lineHeight = 22.sp),
    labelLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.Medium, fontSize = 14.sp, lineHeight = 20.sp),
    labelSmall = TextStyle(fontFamily = FontFamily.SansSerif, fontWeight = FontWeight.Medium, fontSize = 12.sp, lineHeight = 18.sp),
)

@Composable
fun ChainDeskTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (isSystemInDarkTheme()) night else daylight, typography = chainTypography, content = content)
}
