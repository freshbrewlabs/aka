### [1](https://git.freshbrewlabs.cloud/freshbrewlabs/aka/issues/11) - searchable and grouped routes

#### Added

- searchable and grouped routes
- Filter conflict lines with the same search terms so unrelated conflicts are hidden.
- Display the number of host rows that match the active filter compared with all known host rows.
- Require several search terms to match the same route row.
- Keep metric tiles counting all routes when a filter narrows the visible route list.
- Add a route class chip to each grouped host row.
- Group routes by base name (host's last two labels), collapsing them into cards instead of separate protocol tables.
- Show contextual empty-state messages when no routes exist or when none match the search filter.
- Update the routes page header layout to wrap the search input and refresh button on narrow viewports.
