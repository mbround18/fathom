/** Shapes of PEM, not real keys or certificates: the bodies are filler base64. */
const body = 'TUlJQnB6Q0NBVTJnQXdJQkFnSVVaZmlsbGVyZmlsbGVyZmlsbGVyZmlsbGVy\nZmlsbGVyZmlsbGVyZmlsbGVyZmlsbGVyZmlsbGVyZmlsbGVy\n'

export const CERT = `-----BEGIN CERTIFICATE-----\n${body}-----END CERTIFICATE-----\n`
export const KEY = `-----BEGIN PRIVATE KEY-----\n${body}-----END PRIVATE KEY-----\n`
export const CHAIN = CERT + CERT
