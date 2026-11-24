# Documentation build inputs

`legacy-routes.tsv` lists historical uppercase HTML pages and moved directory
pages through their `index.html` paths. Fragment mappings can select a new anchor
or a root-relative page and anchor when a guide is split. The generator validates
these paths and keeps them under the selected GitHub Pages or on-prem mount root.
