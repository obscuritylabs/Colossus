{{- define "cp.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- define "cp.fullname" -}}
{{- if .Values.fullnameOverride -}}{{ .Values.fullnameOverride | trunc 45 | trimSuffix "-" }}
{{- else if contains (include "cp.name" .) .Release.Name -}}{{ .Release.Name | trunc 45 | trimSuffix "-" }}
{{- else -}}{{ printf "%s-%s" .Release.Name (include "cp.name" .) | trunc 45 | trimSuffix "-" }}{{- end -}}
{{- end -}}
{{- define "cp.selector" -}}
app.kubernetes.io/name: {{ include "cp.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}
{{- define "cp.labels" -}}
{{ include "cp.selector" . }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | quote }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}
{{- define "cp.image" -}}
{{- $registry := default .image.registry .root.Values.global.imageRegistry -}}
{{- $name := printf "%s/%s" $registry .image.repository -}}
{{- if .image.digest -}}{{ printf "%s@%s" $name .image.digest }}
{{- else -}}{{ printf "%s:%s" $name (default (printf "v%s" .root.Chart.AppVersion) .image.tag) }}{{- end -}}
{{- end -}}
{{- define "cp.databaseSecret" -}}
{{- default (printf "%s-database" (.Release.Name | trunc 45 | trimSuffix "-")) .Values.global.databaseSecretName -}}
{{- end -}}
{{- define "cp.postgresAdminSecret" -}}
{{- default (printf "%s-postgres-admin" (.Release.Name | trunc 40 | trimSuffix "-")) .Values.global.postgresAdminSecretName -}}
{{- end -}}
{{- define "cp.postgresName" -}}
{{- include "postgres.fullname" .Subcharts.postgres -}}
{{- end -}}
{{- define "cp.stateSecret" -}}
{{- default (printf "%s-state" (include "cp.fullname" .)) .Values.auth.state.existingSecret -}}
{{- end -}}
{{- define "cp.tlsSecret" -}}
{{- default (printf "%s-tls" (include "cp.fullname" .)) .Values.tls.existingSecret -}}
{{- end -}}
{{- define "cp.oidcSecret" -}}
{{- default (printf "%s-oidc" (include "cp.fullname" .)) .Values.auth.oidc.existingSecret -}}
{{- end -}}
{{- define "cp.externalDatabaseSecret" -}}
{{- default (printf "%s-external-db" (include "cp.fullname" .)) .Values.database.external.existingSecret -}}
{{- end -}}
{{- define "cp.databaseCaSecret" -}}
{{- default (printf "%s-database-ca" (include "cp.fullname" .)) .Values.database.external.tls.existingSecret -}}
{{- end -}}
{{- define "cp.openshift" -}}
{{- if or (eq .Values.platform "openshift") (.Capabilities.APIVersions.Has "route.openshift.io/v1") -}}true{{- end -}}
{{- end -}}
{{- define "cp.podSecurityContext" -}}
{{- if include "cp.openshift" . -}}{{ omit .Values.podSecurityContext "runAsUser" "runAsGroup" "fsGroup" | toYaml }}
{{- else -}}{{ toYaml .Values.podSecurityContext }}{{- end -}}
{{- end -}}
{{- define "cp.containerSecurityContext" -}}
{{- if include "cp.openshift" . -}}{{ omit .Values.containerSecurityContext "runAsUser" "runAsGroup" | toYaml }}
{{- else -}}{{ toYaml .Values.containerSecurityContext }}{{- end -}}
{{- end -}}
{{- define "cp.componentSelector" -}}
{{ include "cp.selector" .root }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}
{{- define "cp.spread" -}}
{{- if .root.Values.topologySpread.enabled }}
topologySpreadConstraints:
  - maxSkew: {{ .root.Values.topologySpread.maxSkew }}
    topologyKey: topology.kubernetes.io/zone
    whenUnsatisfiable: {{ .root.Values.topologySpread.whenUnsatisfiable }}
    {{- if eq .root.Values.topologySpread.whenUnsatisfiable "DoNotSchedule" }}
    minDomains: {{ .root.Values.topologySpread.minDomains }}
    {{- end }}
    labelSelector:
      matchLabels:
        {{- include "cp.componentSelector" . | nindent 8 }}
{{- end }}
{{- end -}}
{{- define "cp.meshLabels" -}}
{{- if eq .Values.mesh.mode "ambient" }}
istio.io/dataplane-mode: ambient
sidecar.istio.io/inject: "false"
{{- else if eq .Values.mesh.mode "sidecar" }}
sidecar.istio.io/inject: "true"
{{- end }}
{{- end -}}
