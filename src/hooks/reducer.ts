import { Broadcaster, Broadcasters, Update } from '@/views/List';

type Action =
  | { type: 'SET_LISTS'; online: Broadcaster[]; offline: Broadcaster[] }
  | { type: 'APPLY_UPDATE'; update: Update };

export const reducer = (state: Broadcasters, action: Action): Broadcasters => {
  switch (action.type) {
    case 'SET_LISTS':
      return { online: action.online, offline: action.offline };

    case 'APPLY_UPDATE': {
      const update = action.update;
      const id = update.broadcaster_id || update.broadcaster_id;

      const removeById = (arr: Broadcaster[]) => arr.filter((x) => x.broadcaster_id !== id);

      const inOnline = state.online.find((b) => b.broadcaster_id === id);
      const inOffline = state.offline.find((b) => b.broadcaster_id === id);

      const targetIsOnline = Boolean(update.is_live);

      const mergedFromExisting = (existing?: Broadcaster) => ({
        broadcaster_id: id,
        broadcaster_name: update.broadcaster_name ?? existing?.broadcaster_name ?? '',
        category: update.category ?? existing?.category ?? '',
        title: update.title ?? existing?.title ?? '',
        is_live: Boolean(update.is_live),
        profile_picture: existing?.profile_picture,
      });

      if (targetIsOnline && inOnline) {
        const updatedOnline = state.online.map((b) => (b.broadcaster_id === id ? { ...b, ...update, is_live: true } : b));
        return { ...state, online: updatedOnline };
      }
      if (!targetIsOnline && inOffline) {
        const updatedOffline = state.offline.map((b) =>
          b.broadcaster_id === id ? { ...b, ...update, is_live: false } : b,
        );
        return { ...state, offline: updatedOffline };
      }

      if (targetIsOnline && inOffline) {
        const item = { ...mergedFromExisting(inOffline), is_live: true };
        return {
          online: [item, ...removeById(state.online)],
          offline: removeById(state.offline),
        };
      }
      if (!targetIsOnline && inOnline) {
        const item = { ...mergedFromExisting(inOnline), is_live: false };
        return {
          offline: [item, ...removeById(state.offline)],
          online: removeById(state.online),
        };
      }

      const newItem = { ...mergedFromExisting(undefined), is_live: targetIsOnline };
      if (targetIsOnline) {
        return {
          online: [newItem, ...state.online.filter((b) => b.broadcaster_id !== id)],
          offline: state.offline,
        };
      } else {
        return {
          offline: [newItem, ...state.offline.filter((b) => b.broadcaster_id !== id)],
          online: state.online,
        };
      }
    }

    default:
      return state;
  }
};
