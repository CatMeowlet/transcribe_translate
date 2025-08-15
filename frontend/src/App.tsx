import { useState, useEffect } from 'react';
import { Input } from './components/ui/input';
import { Button } from './components/ui/button';

function App() {
	const _handleGenerateRandomDisplayName = (): string => {
		const adjectives = [
			'Swift',
			'Silent',
			'Mighty',
			'Brave',
			'Lucky',
			'Clever',
			'Shiny',
			'Fierce',
			'Gentle',
			'Epic',
		];
		const nouns = [
			'Tiger',
			'Eagle',
			'Wolf',
			'Panda',
			'Phoenix',
			'Falcon',
			'Shark',
			'Bear',
			'Dragon',
			'Hawk',
		];

		const adjective = adjectives[Math.floor(Math.random() * adjectives.length)];
		const noun = nouns[Math.floor(Math.random() * nouns.length)];
		const number = Math.floor(Math.random() * 1000);

		return `${adjective}${noun}${number}`;
	};

	const [displayName, setDisplayName] = useState<string>('');
	const [room, setRoom] = useState('');
	const [translateTo, setTranslateTo] = useState('en');
	const [transcribeTo, setTranscribeTo] = useState('en');
	const [ws, setWs] = useState<WebSocket | null>(null);

	useEffect(() => {
		setDisplayName(_handleGenerateRandomDisplayName());
	}, []);

	const connectToRoom = () => {
		if (!room.trim()) {
			alert('Please enter a room name.');
			return;
		}

		const params = new URLSearchParams({
			name: displayName,
			translate_to: translateTo,
			transcribe_to: transcribeTo,
		});

		// Pass initial settings in Sec-WebSocket-Protocol (subprotocol)
		const socket = new WebSocket(
			`ws://127.0.0.1:12345/${encodeURIComponent(room)}?${params.toString()}`
		);

		socket.onopen = () => {
			console.log(`Connected to room: ${room} as ${displayName}`);
		};

		socket.onmessage = (event) => {
			console.log('Message from server:', event.data);

			try {
				const data = JSON.parse(event.data);
				if (data.type === 'count') {
					console.log('Participants:', data.count);
				}
			} catch {
				console.warn('Received non-JSON message:', event.data);
			}
		};

		socket.onclose = () => {
			console.log(`Disconnected from room: ${room}`);
		};

		socket.onerror = (err) => {
			console.error('WebSocket error:', err);
		};

		setWs(socket);
	};

	return (
		<div className='max-w-2xl mx-auto py-10'>
			<div className='flex w-full justify-center gap-2'>
				<Input
					value={displayName}
					onChange={(e) => setDisplayName(e.target.value)}
					placeholder='Enter your display name...'
					className='w-64'
				/>
				<Input
					value={room}
					onChange={(e) => setRoom(e.target.value)}
					placeholder='Enter room name...'
					className='w-64'
				/>
				<Input
					value={translateTo}
					onChange={(e) => setTranslateTo(e.target.value)}
					placeholder='Translate to...'
					className='w-32'
				/>
				<Input
					value={transcribeTo}
					onChange={(e) => setTranscribeTo(e.target.value)}
					placeholder='Transcribe to...'
					className='w-32'
				/>
				<Button onClick={connectToRoom}>Connect</Button>
			</div>
		</div>
	);
}

export default App;
