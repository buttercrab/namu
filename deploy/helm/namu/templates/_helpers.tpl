{{/* The release's name, if it says namu already (`namu`, `namu-staging`), or else <release>-namu. */}}
{{- define "namu.fullname" -}}
{{- if contains "namu" .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-namu" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "namu.labels" -}}
app.kubernetes.io/name: namu
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "namu.selector" -}}
app.kubernetes.io/name: namu
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/* The orchestrator's service, as workers and clients reach it. */}}
{{- define "namu.orchestrator" -}}
{{- printf "%s-orchestrator" (include "namu.fullname" .) -}}
{{- end -}}

{{- define "namu.orchestratorUrl" -}}
{{- printf "%s://%s.%s.svc:8080" (ternary "http" "https" .Values.insecure) (include "namu.orchestrator" .) .Release.Namespace -}}
{{- end -}}

{{/* The Secret holding the cluster's TLS: the given one, or cert-manager's. */}}
{{- define "namu.tlsSecret" -}}
{{- if .Values.tls.certManager.enabled -}}
{{- printf "%s-tls" (include "namu.fullname" .) -}}
{{- else -}}
{{- .Values.tls.existingSecret -}}
{{- end -}}
{{- end -}}

{{- define "namu.image" -}}
{{- printf "%s:%s" .image.repository (default .chart.AppVersion .image.tag) -}}
{{- end -}}

{{/* What every NAMU process is given: Redis, TLS, telemetry. */}}
{{- define "namu.commonEnv" -}}
- name: REDIS_URL
  valueFrom:
    secretKeyRef:
      name: {{ .Values.redis.existingSecret }}
      key: {{ .Values.redis.key }}
- name: POD_IP
  valueFrom:
    fieldRef:
      fieldPath: status.podIP
- name: NAMU_ADVERTISE_ADDR
  value: "$(POD_IP):7100"
{{- if .Values.insecure }}
- name: NAMU_INSECURE
  value: "1"
{{- else }}
- name: NAMU_TLS_CA
  value: /etc/namu/tls/ca.crt
- name: NAMU_TLS_CERT
  value: /etc/namu/tls/tls.crt
- name: NAMU_TLS_KEY
  value: /etc/namu/tls/tls.key
{{- end }}
- name: NAMU_LOG_FORMAT
  value: {{ .Values.telemetry.logFormat | quote }}
{{- if .Values.telemetry.metricsPort }}
- name: NAMU_METRICS_ADDR
  value: "0.0.0.0:{{ .Values.telemetry.metricsPort }}"
{{- end }}
{{- with .Values.telemetry.otlpEndpoint }}
- name: OTEL_EXPORTER_OTLP_ENDPOINT
  value: {{ . | quote }}
- name: NAMU_TRACE_RATIO
  value: {{ $.Values.telemetry.traceRatio | quote }}
{{- end }}
{{- end -}}

{{- define "namu.podSecurity" -}}
securityContext:
  runAsNonRoot: true
  runAsUser: 10001
  fsGroup: 10001
  seccompProfile:
    type: RuntimeDefault
{{- end -}}

{{- define "namu.containerSecurity" -}}
securityContext:
  allowPrivilegeEscalation: false
  capabilities:
    drop: ["ALL"]
{{- end -}}
