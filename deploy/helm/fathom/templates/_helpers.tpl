{{- define "fathom.name" -}}{{ .Chart.Name }}{{- end -}}
{{- define "fathom.fullname" -}}{{ .Release.Name }}-{{ .Chart.Name }}{{- end -}}
{{- define "fathom.labels" -}}
app.kubernetes.io/name: {{ include "fathom.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version }}
{{- end -}}
{{- define "fathom.selector" -}}
app.kubernetes.io/name: {{ include "fathom.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
{{- define "fathom.secretName" -}}
{{- if .Values.oidc.existingSecret }}{{ .Values.oidc.existingSecret }}{{ else }}{{ include "fathom.fullname" . }}{{ end -}}
{{- end -}}
