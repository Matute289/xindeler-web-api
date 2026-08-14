# Política de seguridad

`xindeler-web-api` es el backend público de la landing de Xindeler: maneja datos de contacto de
la lista de espera/contribuidores y, a partir de la Fase 2, sesiones web autenticadas. Tomamos en
serio cualquier reporte de vulnerabilidad.

## Cómo reportar

**No abras un issue público.** Usá el reporte privado de GitHub:
["Security" → "Report a vulnerability"](https://github.com/Matute289/xindeler-web-api/security/advisories/new)
en este repo (GitHub Private Vulnerability Reporting).

Incluí, si es posible:
- Descripción del problema y su impacto
- Pasos para reproducirlo
- Versión/commit afectado

## Qué esperar

- Confirmación de recepción en un plazo razonable
- El reporte se coordina de forma privada hasta tener un fix — no se publica ni discute
  públicamente antes de eso
- Crédito en el changelog/advisory si el reporte es válido y así lo preferís

## Alcance

Cubre este repo (`xindeler-web-api`) y su interacción con `xindeler-auth` a través del contrato
público. Vulnerabilidades en `xindeler-auth` o `xindeler-new-horizon` se reportan en sus
respectivos repos.
