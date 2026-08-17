alter table public.fast_explorer_devices
    add column if not exists push_provider text,
    add column if not exists push_token text;

alter table public.fast_explorer_devices
    drop constraint if exists fast_explorer_devices_push_provider_check;
alter table public.fast_explorer_devices
    add constraint fast_explorer_devices_push_provider_check
    check (push_provider is null or push_provider = 'fcm');

alter table public.fast_explorer_devices
    drop constraint if exists fast_explorer_devices_push_token_length_check;
alter table public.fast_explorer_devices
    add constraint fast_explorer_devices_push_token_length_check
    check (push_token is null or char_length(push_token) <= 4096);

comment on column public.fast_explorer_devices.push_token is
    'FCM registration token used only by the server-side FastExplorer push function.';
