---
name: slate-mail-thunderbird
description: Email, calendar and contacts with Thunderbird on SlateOS: read and search mail, compose and send (with attachments), create calendar events, look up contacts. Use for any request about mail, inbox, sending a file to someone, meetings or appointments.
---

# Mail, calendar and contacts with Thunderbird

Thunderbird is the mail, calendar and address book app. Accounts are the user's: if none is configured (`~/.thunderbird` has no account, or the window shows the account setup page), say so and stop; setting up an account needs the user's credentials, which you never ask for in chat.

## Compose (the fastest path)

```
thunderbird -compose "to='alice@example.com',subject='Report',body='Attached.',attachment='/home/me/Documents/report.pdf'"
```

Several addresses: `to='a@x,b@y'`; also `cc=`, `bcc=`. The compose window opens with everything filled in. Tier: reversible (nothing is sent). Check the window with `desktop_elements` (fields are named "To", "Subject", the body is a document), fix anything, then **sending is tier Confirm**: ask the user, then `desktop_element_click` the "Send" button (or Ctrl+Return with the window set). A sent mail cannot be undone.

## Read and search

- Open Thunderbird (`desktop_launch thunderbird`). The message list, folders and the reading pane are all elements: `desktop_read` on the main window returns the visible subjects and the open message's text; `desktop_element_click` a message to open it.
- Search: the "Search" or "Quick Filter" entry is an element; `desktop_element_set_text` it and press Return with the window set.
- Attachments: in the open message, attachments are named elements; "Save" via the attachment's menu, or `desktop_element_click` the attachment then the Save button in the dialog.

Tier: observe for reading; Confirm before deleting mail or moving it to Trash.

## Calendar

Open the Calendar tab (element "Calendar" in the tab bar, or Ctrl+Shift+C with the window set). "New Event" is a button; the dialog's Title, Location, start and end fields are elements: fill them with `desktop_element_set_text`, then "Save and Close". Tier: reversible.

## Contacts

Address Book (Ctrl+Shift+B): "New Contact" button, fields as elements. Looking up an address: `desktop_element_set_text` the search entry.

## Verify

After sending, the Sent folder lists the message; after creating an event, `desktop_read` on the calendar shows it. Report what you saw, not what you intended.
