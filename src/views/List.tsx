import { Broadcaster } from '@/components/Broadcaster';
import { Live } from '@/components/Live';
import { Separator } from '@/components/ui/separator';
import { Spinner } from '@/components/ui/spinner';
import { reducer } from '@/hooks/reducer';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { Dispatch, SetStateAction, useEffect, useReducer } from 'react';

export type Broadcaster = {
  broadcaster_id: string;
  broadcaster_name: string;
  category: string;
  title: string;
  is_live: boolean;
  profile_picture?: string;
};

export type Broadcasters = {
  online: Broadcaster[];
  offline: Broadcaster[];
};

export type Update = {
  broadcaster_id: string;
  broadcaster_name: string;
  category: string;
  title: string;
  is_live: boolean;
  type: string;
  updated: string;
};

export type BroadcastUpdate = {
  broadcaster_id: string;
  payload: Update;
  sub_id: string;
};

interface IListProps {
  loading: boolean;
  setLoading: Dispatch<SetStateAction<boolean>>;
}

export const List = ({ loading, setLoading }: IListProps) => {
  const [state, dispatch] = useReducer(reducer, { online: [], offline: [] });

  useEffect(() => {
    let unlistenStreamers: UnlistenFn;
    listen('streamers:fetched', (event) => {
      const { offline, online } = event.payload as Broadcasters;
      dispatch({ type: 'SET_LISTS', online: online, offline: offline });
      setLoading(false);
    }).then((fn) => {
      unlistenStreamers = fn;
    });
    return () => {
      unlistenStreamers && unlistenStreamers();
    };
  }, []);

  useEffect(() => {
    let unlistenUpdates: UnlistenFn;
    listen('streamer:update', (event) => {
      const { payload } = event.payload as BroadcastUpdate;
      dispatch({
        type: 'APPLY_UPDATE',
        update: payload,
      });
    }).then((fn) => {
      unlistenUpdates = fn;
    });
    return () => {
      unlistenUpdates && unlistenUpdates();
    };
  });

  return (
    <div className="w-full overflow-x-hidden">
      {loading ? (
        <div className="flex flex-row justify-center">
          <Spinner variant="infinite" size={64} />
        </div>
      ) : (
        <div id="broadcasters" className="flex flex-col ml-2">
          {state.online
            .sort((a, b) => a.broadcaster_name.localeCompare(b.broadcaster_name))
            .map((streamer) => (
              <div key={streamer.broadcaster_id} className="flex w-full">
                <div className="flex-1 min-w-0">
                  <Broadcaster {...streamer} />
                </div>
                <div className="flex-shrink-0 content-center mr-2 mb-3 mt-2">
                  <Live />
                </div>
              </div>
            ))}
          <Separator />
          {state.offline
            .sort((a, b) => a.broadcaster_name.localeCompare(b.broadcaster_name))
            .map((streamer) => (
              <Broadcaster key={streamer.broadcaster_id} {...streamer} />
            ))}
        </div>
      )}
    </div>
  );
};
