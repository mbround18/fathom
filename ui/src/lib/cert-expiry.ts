/** Formats a signed seconds-until-expiry value from `CertificateSummary.expiry_seconds`. */
export function formatExpiryCountdown(expirySeconds: number | null): string {
  if (expirySeconds === null) return 'unknown'
  const days = Math.floor(Math.abs(expirySeconds) / 86400)
  if (expirySeconds < 0) return `expired ${String(days)}d ago`
  return `expires in ${String(days)}d`
}
