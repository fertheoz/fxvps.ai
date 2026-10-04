{{- define "fxvps.labels" -}}
app.kubernetes.io/name: {{ .name }}
app.kubernetes.io/instance: {{ .root.Release.Name }}
app.kubernetes.io/part-of: fxvps
app.kubernetes.io/managed-by: {{ .root.Release.Service }}
helm.sh/chart: {{ .root.Chart.Name }}-{{ .root.Chart.Version }}
{{- end -}}
{{- define "fxvps.selector" -}}
app.kubernetes.io/name: {{ .name }}
app.kubernetes.io/instance: {{ .root.Release.Name }}
{{- end -}}
{{- define "fxvps.probe" -}}
{{- if eq .probes.type "http" }}
httpGet: { path: {{ .path }}, port: {{ .probes.port }} }
{{- else if eq .probes.type "tcp" }}
tcpSocket: { port: {{ .probes.port }} }
{{- end }}
{{- end -}}
